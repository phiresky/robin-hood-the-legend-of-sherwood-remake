"""Bounded native contact-color fallback on the exact approved flat receiver."""
import sys,json,argparse
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
from restart3_initial_fence_contact import link
from tree_geometry import SIN,RAY
from review_bank_candidate import camera
from refinement_review import _tree
BASE=OUT/'restart4-fence14-ground-candidate-v1/model.blend'
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--exclude-tree19-tip',action='store_true');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]if '--'in sys.argv else [])
 excluded=[[117,918]]+([[1594,147]]if args.exclude_tree19_tip else[]);count=51-len(excluded);d=OUT/f'restart4-fringe{count}-source-contact-v1';assert not d.exists();assert sha(BASE)=='4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8'
 authority=OUT/'restart7-underlay-role/fringe51-v2/report.json';proof=json.loads(authority.read_text());rows=[r for r in proof['rows']if r['variant']==('approved-foreground'if r['mask']==95 else'pending-root-geometry')and r['pixel']not in excluded];assert len(rows)==count and all(r['ground_first_hit']and r['covered_and_initial_source_equal']for r in rows)
 sourcepath=OUT/'source-states/covered.png';src=np.array(Image.open(sourcepath).convert('RGBA'));bpy.ops.wm.open_mainfile(filepath=str(BASE));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update();ground=bpy.data.objects['Croisement02 Terrain'];sig=geometry(ground);node,old=atlas(ground);new=old.copy();domain=np.zeros(old.shape[:2],bool)
 for r in rows:x,y=r['pixel'];domain[y,x]=True;new[y,x]=src[y,x]
 known=np.array(Image.open(OUT/'restart4-final-floor-bake-v1/known-native-domain.png').convert('L'))>0;prior14=np.array(Image.open(BASE.parent/'source-context14-domain.png').convert('L'))>0
 assert domain.sum()==count and not(domain&(known|prior14)).any()and np.array_equal(new[~domain],old[~domain])and np.array_equal(new[:,:,3],old[:,:,3]);d.mkdir();Image.fromarray(new).save(d/'composite.png');Image.fromarray(domain.astype('uint8')*255).save(d/'domain.png');im=bpy.data.images.load(str(d/'composite.png'),check_existing=False);im.pack();node.image=im;bpy.ops.wm.save_as_mainfile(filepath=str(d/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(d/'model.blend'));bpy.context.view_layer.update();ground=bpy.data.objects['Croisement02 Terrain'];assert geometry(ground)==sig and np.array_equal(atlas(ground)[1],new)
 write_json(d/'validation.json',dict(status='PASS private bounded source-contact appearance; user approval pending',model_sha256=sha(d/'model.blend'),base_sha256=sha(BASE),source_sha256=sha(sourcepath),authority_sha256=sha(authority),atlas_sha256=sha(d/'composite.png'),changed_pixels=count,exact_native_source_rgba=count,all_outside_rgba_exact=int((~domain).sum()),prior_known772251_exact=True,prior14_exact=True,alpha_exact=True,geometry_uv_exact=True,geometry_signature=sig,packed_atlas_reopened_exact=True,excluded_elevated_samples=excluded,ownership='Conservative2D native contact colors only; physical wood/ground role remains uncertain',state_precedence='Covered source is raw Day background. All initial/transition FX and terminal patches remain separate and above this static fallback; no state asset changed.',api_calls=0))
 contacts=[]
 for number in [19,25,95]:
  selected=[r for r in rows if r['mask']==number];coords=[r['pixel']for r in selected];folder=d/f'mask{number}';folder.mkdir();paths=[Path(p)for p in proof['pins']if (f'tree{number}-toe-finished-v12' in p if number!=95 else'approved-fence95-fill-v1' in p)];assert len(paths)==1;path=paths[0];assert sha(path)==proof['pins'][str(path)];asset=f'croisement02-tree-{number}'if number!=95 else'croisement02-east-upright-rail-fence-95'
  bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objs=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset];frozen={o.name:geometry(o)for o in objs};bpy.ops.wm.open_mainfile(filepath=str(d/'model.blend'));scene=bpy.data.scenes.new(f'Contact colors {number}');bpy.context.window.scene=scene;ground=bpy.data.objects['Croisement02 Terrain'];link(scene,ground);ground.hide_render=False
  with bpy.data.libraries.load(str(path),link=False)as(s,t):t.objects=list(frozen)
  for o in t.objects:link(scene,o);o.hide_render=False
  bpy.context.view_layer.update();assert all(geometry(o)==frozen[o.name]for o in t.objects);tree,owners,_=_tree([ground]+t.objects)
  for x,y in coords:
   p,_,i,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);assert owners[i]==ground;contacts.append(dict(mask=number,pixel=[x,y],receiver=ground.name))
  box=json.loads((OUT/f'restart7-underlay-role/fringe51-v2/mask{number}/crop.json').read_text())['box'];center=Vector(((box[0]+box[2])/2,-(box[1]+box[3])/2/SIN,0));scale=box[2]-box[0];node,_=atlas(ground);after=node.image;before=bpy.data.images.load(str(BASE.parent/'composite.png'),check_existing=False)
  for view,direction in [('native',RAY),('opposite',Vector((-RAY.x,-RAY.y,RAY.z)))]:
   camera(scene,center,direction,512,512,scale);scene.cycles.transparent_max_bounces=512;scene.cycles.samples=8
   for label,image in [('before',before),('after',after)]:node.image=image;scene.render.filepath=str(folder/f'{view}-{label}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
  node.image=after;sheet=Image.new('RGB',(1024,1080),'#292929');draw=ImageDraw.Draw(sheet)
  for row,view in enumerate(['native','opposite']):
   for col,label in enumerate(['before','after']):sheet.paste(Image.open(folder/f'{view}-{label}.png'),(col*512,row*540+28));draw.text((col*512+5,row*540+7),view+' '+label,fill='white')
  sheet.save(folder/'paired.png');marked=src.copy()
  for x,y in coords:marked[y,x,:3]=[255,30,150]
  sheet=Image.new('RGB',(1536,540),'#292929');draw=ImageDraw.Draw(sheet)
  for col,(a,label)in enumerate([(src,'Native source'),(marked,'Selected contact colors'),(new,'Candidate ground atlas')]):sheet.paste(Image.fromarray(a).transform((512,512),Image.Transform.EXTENT,box,Image.Resampling.NEAREST),(col*512,28));draw.text((col*512+5,7),label,fill='white')
  sheet.save(folder/'source.png')
 write_json(d/'context-validation.json',dict(status='PASS saved candidate native first hits',model_sha256=sha(d/'model.blend'),rows=contacts,source_models_unchanged=all(sha(Path(p))==h for p,h in proof['pins'].items()),native_camera_first=True,opposite_direction=list(Vector((-RAY.x,-RAY.y,RAY.z))),new_model_saved_only_once=True))
 print('DONE',count,sha(d/'model.blend'),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()

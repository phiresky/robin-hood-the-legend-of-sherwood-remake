"""Read-only native fringe versus approved ground14, with exact local receivers."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN
from restart3_fence_receiver import atlas
from restart3_initial_fence_contact import link
from restore_ground75_source import geometry
from review_bank_candidate import camera
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
DEST=OUT/'restart7-underlay-role/fringe51-v2';GROUND=OUT/'restart4-fence14-ground-candidate-v1/model.blend';GROUND_SHA='4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8'

def main():
 assert shutil.disk_usage(OUT).free>25*2**30;assert sha(GROUND)==GROUND_SHA;DEST.mkdir(parents=True,exist_ok=False);authority=ROOT/'source-role132-v1/private-toe-residual-v12.json';residual=json.load(open(authority));coords={r['mask']:[v['pixel']for v in r['rows']if not v['center_covered']and not v['positive_subsamples_of25']]for r in residual['records']};coords[95]=next(r['coordinates']for r in json.load(open(ROOT/'remaining-inventory-v1/report.json'))['rows']if r['mask']==95);source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));covered=np.array(Image.open(OUT/'source-states/covered.png').convert('RGBA'));known=np.array(Image.open(OUT/'restart4-final-floor-bake-v1/known-native-domain.png').convert('L'))>0;records=[];pins={str(GROUND):GROUND_SHA};transforms=[]
 for number in [19,25,95]:
  asset=f'croisement02-tree-{number}'if number!=95 else'croisement02-east-upright-rail-fence-95';old=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));variants=[('approved-foreground',Path(old['source']),old['source_sha256'])]
  if number!=95:
   model=ROOT/f'tree{number}-toe-finished-v12/model.blend';variants.append(('pending-root-geometry',model,sha(model)))
  target=coords[number];a=np.array(target);lo=a.min(0)-18;hi=a.max(0)+19;center=(lo+hi)/2;scale=float(max(hi-lo));box=[center[0]-scale/2,center[1]-scale/2,center[0]+scale/2,center[1]+scale/2];folder=DEST/f'mask{number}';folder.mkdir();base=None
  for label,path,digest in variants:
   assert sha(path)==digest;pins[str(path)]=digest;bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objs=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset];expected={o.name:dict(matrix=np.array(o.matrix_world),signature=geometry(o))for o in objs};names=list(expected);bpy.ops.wm.open_mainfile(filepath=str(GROUND));bpy.context.view_layer.update();scene=bpy.data.scenes.new(f'Readonly fringe {number} {label}');bpy.context.window.scene=scene;ground=bpy.data.objects['Croisement02 Terrain'];link(scene,ground);ground.hide_render=False;bpy.context.view_layer.update();sig=geometry(ground);_,base=atlas(ground)
   with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.objects=list(names)
   context=[o for o in dst.objects if o]
   for o in context:link(scene,o);o.hide_render=False
   bpy.context.view_layer.update()
   for o,original_name in zip(context,names):
    assert np.array_equal(np.array(o.matrix_world),expected[original_name]['matrix']);assert geometry(o)==expected[original_name]['signature'];transforms.append(dict(model=str(path),object=original_name,evaluated_world_transform_exact=True,geometry_exact=True))
   objects=[ground]+context;tree,owners,_=_tree(objects);ground.data.calc_loop_triangles();gtris=list(ground.data.loop_triangles);uv=ground.data.uv_layers.active
   for x,y in target:
    p,n,index,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);owner=owners[index]if p is not None else None;atlas_pixel=None;sample=None
    if owner==ground:
     t=gtris[index];q=barycentric_transform(p,*[ground.matrix_world@ground.data.vertices[i].co for i in t.vertices],*[Vector((*uv.data[li].uv,0))for li in t.loops]);ax=min(1791,max(0,int(q.x*1792)));ay=min(1151,max(0,1151-int(q.y*1152)));atlas_pixel=[ax,ay];sample=base[ay,ax].tolist()
    records.append(dict(mask=number,variant=label,pixel=[x,y],receiver=owner.name if owner else None,ground_first_hit=owner==ground,hit=list(p)if p is not None else None,source_rgba=source[y,x].tolist(),covered_rgba=covered[y,x].tolist(),covered_and_initial_source_equal=bool(np.array_equal(source[y,x],covered[y,x])),ground_atlas_xy=atlas_pixel,current_first_hit_ground_rgba=sample,ground_native_xy_rgba=base[y,x].tolist(),ground_matches_source=bool(sample==source[y,x].tolist())if sample else None,known_native_ground=bool(known[y,x])))
   camera(scene,Vector((center[0],-center[1]/SIN,0)),RAY,512,512,scale);scene.cycles.transparent_max_bounces=512;scene.cycles.samples=8;scene.render.filepath=str(folder/f'{label}-native.png');bpy.ops.render.render(write_still=True,scene=scene.name);assert geometry(ground)==sig and np.array_equal(atlas(ground)[1],base);assert sha(path)==digest
  marked=source.copy();markedbase=base.copy()
  for x,y in target:marked[y,x,:3]=[255,30,150];markedbase[y,x,:3]=[255,30,150]
  panel=Image.new('RGB',(1536,412),'#292929');draw=ImageDraw.Draw(panel)
  for i,(image,title)in enumerate([(source,'Original initial source'),(marked,'Target context centers'),(base,'Approved ground14 native atlas'),(markedbase,'Same centers on ground')]):
   crop=Image.fromarray(image).transform((384,384),Image.Transform.EXTENT,box,Image.Resampling.NEAREST);panel.paste(crop,(384*i,28));draw.text((384*i+4,7),title,fill='white')
  panel.save(folder/'source-versus-underlay.png');Image.fromarray(source).transform((512,512),Image.Transform.EXTENT,box,Image.Resampling.NEAREST).save(folder/'original-source.png');write_json(folder/'crop.json',dict(box=box,target_centers=target,native_first=True))
 assert sha(GROUND)==GROUND_SHA;write_json(DEST/'report.json',dict(status='Read-only source-role evidence; no appearance or ownership change',pins=pins,authority_sha256=sha(authority),targets={str(k):len(v)for k,v in coords.items()},rows=records,context_transform_checks=transforms,scope='Local ground plus exact own foreground only; prior whole-map first-hit audit corroborates original ground exposure. Pending root variants remain separate from approved scene. No model saved, no material edit, no API. Source RGB restoration is a possible conservative2D fallback, not physical wood or broad ground ownership.',files={str(p.relative_to(DEST)):sha(p)for p in DEST.rglob('*.png')}))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()

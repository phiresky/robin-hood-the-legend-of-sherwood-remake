"""Prepare a bounded same-receiver continuation beneath native trap phases."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,RAY
from render_slots import acquire,release
BASE=OUT/'restart2-state';DEST=BASE/'trap-ground-continuation-v1'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 atlas_path=OUT/'restart3-fence-receiver/terminal-v3/base-atlas.png';atlas=np.array(Image.open(atlas_path).convert('RGBA'));result=atlas.copy()
 domain=BASE/'trap-underlay-domain-audit-v1';foreign=np.array(Image.open(domain/'foreign-static.png'))>0;trap=np.array(Image.open(domain/'trap-union.png'))>0
 target=np.array(Image.open(BASE/'trap-underlay-receivers-v1/receiver-00.png'))>0;assert target.sum()==14
 allowed=~foreign&~trap&~np.all(atlas[:,:,:3]==127,axis=2)&(atlas[:,:,3]==255)
 ledger=json.loads((BASE/'receiver-rebind-v2/report.json').read_text());acquire()
 try:
  DEST.mkdir();vertices=[];faces=[];owners=[];objects=[]
  for model in ledger['models']:
   path=Path(model['path']);assert sha(path)==model['sha256'];bpy.ops.wm.open_mainfile(filepath=str(path))
   if model['receiver']=='bank':bpy.context.window.scene=bpy.data.scenes[json.loads((path.parent/'workspace.json').read_text())['scene_name']]
   bpy.context.view_layer.update()
   for row in ledger['objects']:
    if row['receiver']!=model['receiver']:continue
    obj=bpy.data.objects[row['name']];assert np.max(abs(np.array(obj.matrix_world)-np.array(row['matrix_world'])))<1e-6;obj.data.calc_loop_triangles();offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);faces.extend(tuple(offset+i for i in t.vertices)for t in obj.data.loop_triangles);owners.extend([len(objects)]*len(obj.data.loop_triangles));objects.append(row)
  tree=BVHTree.FromPolygons(vertices,faces,all_triangles=True);cache={}
  def owner(x,y):
   key=(x,y)
   if key not in cache:
    hit,_,face,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000,-RAY);cache[key]=None if hit is None else owners[face]
   return cache[key]
  samples=[]
  for y,x in np.argwhere(target):
   x=int(x);y=int(y);assert owner(x,y)==0;candidates=[]
   for dy in range(-24,25):
    for dx in range(-24,25):
     xx=x+dx;yy=y+dy
     if 0<=yy<1152 and 0<=xx<1792 and allowed[yy,xx]:candidates.append((dx*dx+dy*dy,yy,xx))
   picked=None
   for distance,yy,xx in sorted(candidates):
    if owner(xx,yy)==0:picked=(distance,yy,xx);break
   if picked is None:raise ValueError(f'No bounded same-ground sample for {x},{y}')
   distance,yy,xx=picked;result[y,x]=atlas[yy,xx];samples.append({'target':[x,y],'sample':[xx,yy],'distance':distance**.5,'rgba':atlas[yy,xx].tolist(),'target_receiver':0,'sample_receiver':0,'sample_outside_all_trap_and_foreign_masks':True})
  assert np.array_equal(result[~target],atlas[~target]);assert np.array_equal(result[:,:,3],atlas[:,:,3]);assert np.sum(np.any(result!=atlas,axis=2))==14
  Image.fromarray(result).save(DEST/'proposed-atlas.png');Image.fromarray(target.astype('uint8')*255).save(DEST/'editable.png')
  crop=(250,340,355,405);sheet=Image.new('RGB',(840,260),'#333333')
  for i,array in enumerate([atlas,result]):sheet.paste(Image.fromarray(array).crop(crop).resize((420,260),Image.Resampling.NEAREST).convert('RGB'),(420*i,0))
  sheet.save(DEST/'before-after.png');annotated=Image.fromarray(atlas).convert('RGB');draw=ImageDraw.Draw(annotated)
  for sample in samples:draw.line([tuple(sample['target']),tuple(sample['sample'])],fill=(0,255,255));draw.point(tuple(sample['sample']),fill=(255,0,255))
  annotated.crop(crop).resize((840,520),Image.Resampling.NEAREST).save(DEST/'sample-authority.png')
  write_json(DEST/'report.json',{'status':'Private bounded inferred atlas proposal; no base model mutation','base_atlas':str(atlas_path),'base_atlas_sha256':sha(atlas_path),'proposed_atlas_sha256':sha(DEST/'proposed-atlas.png'),'changed_pixels':14,'outside_exact':True,'alpha_exact':True,'receiver_models':ledger['models'],'samples':samples,'phase_coverage_evidence':str(BASE/'trap-underlay-receivers-v1/ground-fourteen-phases.json'),'limits':['Inferred continuation from nearest accepted same-ground pixel; not new native evidence.','Samples exclude every trap and foreign scenery reservation and require current physical ground first-hit.','Native dynamic images and approved bank materials untouched.','Geometry and UV unchanged because no model is edited; context review required before material integration.']})
 finally:release()
if __name__=='__main__':main()

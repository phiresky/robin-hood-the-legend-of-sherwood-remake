"""Independent native mask and source-pixel coverage for private wood geometry."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement';OUT=B/'restart2/tree13-wood-v6/source-coverage';RAY=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))))
def main():
 OUT.mkdir(exist_ok=False);model=OUT.parent/'worker.blend';level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());box=(1030,0,1110,112);source=Image.open(B/'baseline/covered.png').convert('RGB');native=Image.new('L',source.size);native.paste(Image.open(B/'baseline/masks/000013.png'),tuple(level['masks'][13]['box_top_left']));mask=np.array(native.crop(box))>0;known=np.array(Image.open(B/'restart2/tree13-bark-proposal-v1/proposed-bark.png').crop(box))>0;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-13'];assert len(objects)==3;trees=[(o,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons])) for o in objects];hit=np.zeros(mask.shape,bool);owners=np.zeros(mask.shape,int);bounds={}
  for o,t in trees:
   ps=[o.matrix_world@v.co for v in o.data.vertices];bounds[o['source_node']]=[[min(p[i] for p in ps),max(p[i] for p in ps)] for i in range(3)]
  for yy in range(mask.shape[0]):
   for xx in range(mask.shape[1]):
    x,y=box[0]+xx,box[1]+yy;origin=Vector((x+.5,-(y+.5)/math.sin(math.radians(35)),0))+RAY*10000;rows=[]
    for o,t in trees:
     p,n,f,d=t.ray_cast(origin,-RAY)
     if p is not None:rows.append((d,int(o['source_node'].rsplit('-',1)[1])))
    if rows:rows.sort();hit[yy,xx]=True;owners[yy,xx]=rows[0][1]
  assert np.all(hit[known]);rgb=np.array(source.crop(box));geom=np.full_like(rgb,52);geom[hit]=[155,155,155];geom[known]=rgb[known];classes=rgb.copy();classes[hit&~mask]=[0,140,255];classes[mask&~hit]=[230,70,70];classes[known]=[0,255,120];ims=[Image.fromarray(a).resize((320,448),Image.Resampling.NEAREST) for a in (rgb,geom,classes)];sheet=Image.new('RGB',(960,476),'#333333');draw=ImageDraw.Draw(sheet)
  for i,(im,label) in enumerate(zip(ims,['Original native art','Wood ray geometry; known RGB','Green153 / blue outside / red mixed gap'])):sheet.paste(im,(i*320,28));draw.text((i*320+5,8),label,fill='white')
  sheet.save(OUT/'comparison.png');write_json(OUT/'receipt.json',dict(model_sha256=sha(model),source_sha256=sha(B/'baseline/covered.png'),native_mask_sha256=sha(B/'baseline/masks/000013.png'),source_bbox=list(box),source_camera='Original native pixel rays at35 degrees',known_pixels=int(known.sum()),known_misses=int((known&~hit).sum()),native_mixed_mask_pixels=int(mask.sum()),native_mixed_pixels_not_hit=int((mask&~hit).sum()),wood_projection_outside_mixed_mask=int((hit&~mask).sum()),source_projected_wood_pixels=int(hit.sum()),world_bounds=bounds,sheet_sha256=sha(OUT/'comparison.png'),limits=['153 accepted bark cores are the only positive RGB ownership.','Red native-mask gaps may be leaf/ground/wood and remain unresolved, not silently accepted as absent bark.','Blue geometric projection outside mixed native mask requires boundary/source-context review.','Upper continuation lies outside native raster; measured separately in world bounds.']))
 finally:release()
if __name__=='__main__':main()

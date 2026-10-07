"""Resolve measured native leaf/stem ordering without changing source RGB or UV."""
import sys,math,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
def main():
 src=B/'tree05-crown-prototype-v4';out=B/'tree05-crown-prototype-v5';assert shutil.disk_usage(ROOT).free>25*1024**3;out.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;fol=next(o for o in scene.objects if o.get('asset_group')=='croisement03-arbre07-fragment-tree05-provisional');r=json.loads((src/'receipt.json').read_text());wood=[o for o in scene.objects if o.type=='MESH' and o!=fol];trees=[BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]) for o in wood];union=np.logical_or.reduce([np.array(Image.open(p))[:,:,3]>0 for p in sorted((B/'tree05-canopy-fragment-source-v1').glob('???.png'))]);moves=[]
  for f in list(fol.data.polygons)[:r['native_faces']]:
   vs=[fol.data.vertices[i].co.copy() for i in f.vertices];bounds=[(v.x,-v.y*SIN-v.z*COS) for v in vs];xs=[p[0] for p in bounds];ys=[p[1] for p in bounds];delta=0
   for y in range(max(0,math.floor(min(ys)+.01)),min(154,math.ceil(max(ys)-.01))):
    for x in range(max(492,math.floor(min(xs)+.01)),min(571,math.ceil(max(xs)-.01))):
     if not union[y,x-492]:continue
     leaf=Vector((x+.5,vs[0].y,(-vs[0].y*SIN-y-.5)/COS));origin=leaf+RAY*10000
     for t in trees:
      p,n,fi,d=t.ray_cast(origin,-RAY)
      if p is not None:delta=max(delta,10000-d+.05)
   if delta>0:
    for i in f.vertices:fol.data.vertices[i].co+=RAY*delta
    moves.append(dict(face=f.index,source_ray_offset=delta))
  support=next(o for o in scene.objects if o.name=='Inferred cluster support 4');start=[v.co.copy() for v in support.data.vertices[:8]]
  for v in support.data.vertices[8:]:v.co-=RAY*8
  assert all((a-b.co).length==0 for a,b in zip(start,support.data.vertices[:8]));fol.data.update();support.data.update();bpy.data.libraries.write(str(out/'worker.blend'),{scene},fake_user=True,compress=True)
  render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    p=Image.open(out/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',p.size,'#333333');bg.alpha_composite(p);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(out/'actual'/f'{mode}.png')
  r.update(status='PRIVATE measured source-ray clearance; full source and joint validation pending',model_sha256=sha(out/'worker.blend'),previous_model_sha256=sha(src/'worker.blend'),native_face_ray_clearance=moves,support4_unknown_mid_end_ray_offset=-8,support4_attachment_ring_exact=True);write_json(out/'receipt.json',r);print('MOVED_NATIVE_CELLS',len(moves))
 finally:release()
if __name__=='__main__':main()

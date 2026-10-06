"""Fit three circular stem profiles to separate native silhouette components."""
import json,sys,math,shutil,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_tree13_wood_v1 import mesh_for
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree13-wood-v5';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
def components(a):
 seen=set();result=[]
 for y,x in zip(*np.nonzero(a)):
  key=(int(y),int(x))
  if key in seen:continue
  seen.add(key);stack=[key];ps=[]
  while stack:
   yy,xx=stack.pop();ps.append((xx+1036,yy))
   for dy,dx in ((0,1),(0,-1),(1,0),(-1,0)):
    q=(yy+dy,xx+dx)
    if 0<=q[0]<a.shape[0] and 0<=q[1]<a.shape[1] and a[q] and q not in seen:seen.add(q);stack.append(q)
  if len(ps)>20:result.append(ps)
 return sorted(result,key=lambda ps:min(x for x,y in ps))
def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);source=B/'tree13-wood-v3/worker.blend';masks=components(np.array(Image.open(B.parent/'baseline/masks/000013.png'))>0);assert len(masks)==3;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;objects=[o for o in scene.objects if o.type=='MESH'];prior_images={im.name:hashlib.sha256(np.array(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};records=[];projections={r['node']:r for r in json.loads((source.parent/'receipt.json').read_text())['records']}
  for node,ps in zip((48,31,32),masks):
   rows={y:[x for x,yy in ps if yy==y] for y in sorted({y for x,y in ps})};profile={y:((min(xs)+max(xs)+1)/2,(max(xs)-min(xs)+1)/2+.15) for y,xs in rows.items()};last=max(rows);base_y=last-profile[max(0,last-3)][1]*SIN;ground=base_y;levels=sorted({0,int(base_y),*range(0,int(base_y)+1,3)},reverse=True);points=[]
   for y in levels:
    x,r=profile[y]
    if node==32:r+=.4*max(0,1-abs(y-68)/7)
    points.append((x,float(y),r))
   points[0]=(points[0][0],ground,points[0][2]);x0,r0=profile[0];slope=max(-.12,min(.12,(profile[min(20,last)][0]-x0)/20));
   for y,factor in ((-35,.9),(-80,.7),(-125,.45),(-165,.1)):points.append((x0+slope*y,float(y),max(.45,r0*factor)))
   obj=next(o for o in objects if o['source_node']==f'building-{node:03}');materials=list(obj.data.materials);slot=len(materials)-1;mesh=mesh_for(f'Native-fitted circular stem{node}',dict(ground=ground,points=points));mesh.materials.clear()
   for mat in materials:mesh.materials.append(mat)
   obj.data=mesh;obj.matrix_world.identity();uv=mesh.uv_layers.new(name=f'Observed bark {node:03}');left,top,right,bottom=projections[node]['crop'];w,h=right-left,bottom-top
   for f in mesh.polygons:
    f.material_index=slot
    for li in f.loop_indices:
     p=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((p.x-left)/w,1-((-p.y*SIN-p.z*COS)-top)/h)
   t=BVHTree.FromPolygons([v.co for v in mesh.vertices],[list(f.vertices) for f in mesh.polygons]);known=np.array(Image.open(B/f'tree13-bark-proposal-v1/node-{node:03}-proposed-bark.png'))>0;miss=[]
   for y,x in zip(*np.nonzero(known)):
    p,n,f,d=t.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000,-RAY)
    if p is None:miss.append([int(x),int(y)])
   records.append(dict(node=node,ground_source_y=ground,points=points,known_pixels=int(known.sum()),known_misses=miss,source_component_pixels=len(ps)))
  assert prior_images=={im.name:hashlib.sha256(np.array(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.name in prior_images};bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);views={f'view-{i}':f'Tree13 view{i}' for i in range(8)};render_views(scene.name,views,OUT/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(OUT/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(OUT/'actual'/f'{mode}.png')
  write_json(OUT/'receipt.json',dict(status='PRIVATE native profile geometry; full source/joint audit pending',model_sha256=sha(OUT/'worker.blend'),previous_model_sha256=sha(source),all_existing_image_rgba_exact=True,records=records,actual8_sha256=sha(OUT/'actual/textured.png'),limits=['Native mask components constrain inferred wood envelopes; mask interiors are still mixed/unassigned except accepted153bark.','Off-map continuation extrapolates bounded local source lean and tapers; crown is separate.','Three separate circular stems, not a flattened single-tree volume.']));print([(r['node'],len(r['known_misses'])) for r in records])
 finally:release()
if __name__=='__main__':main()

"""Remove inferred leaf cards that cover reserved neighboring bark rays."""
import sys,math,json,shutil,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;src=B/'tree03-crown-prototype-v1';out=B/'tree03-crown-prototype-v2';out.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];leaf=next(o for o in scene.objects if o.get('asset_group')=='croisement03-arbre08-fragment-tree03-provisional');m=leaf.data;d=json.loads((src/'receipt.json').read_text());native=d['native_faces'];reserved=np.array(Image.open(B/'tree04-bark-proposal-v1/proposed-bark.png'))>0;y,x=np.nonzero(reserved);points=np.column_stack((x+.5,y+.5));keep=[];removed=[]
  for f in m.polygons:
   q=np.array([(m.vertices[i].co.x,-m.vertices[i].co.y*SIN-m.vertices[i].co.z*COS) for i in f.vertices]);hit=np.any(np.all((points>=q.min(axis=0)-.01)&(points<=q.max(axis=0)+.01),axis=1))
   if f.index>=native and hit:removed.append(f.index)
   else:keep.append(f)
  assert removed and len(removed)<len(m.polygons)*.15
  mesh=bpy.data.meshes.new(m.name+' neighbor-preserving');mesh.from_pydata([tuple(v.co) for v in m.vertices],[],[list(f.vertices) for f in keep]);mesh.update()
  for mat in m.materials:mesh.materials.append(mat)
  for layer in m.uv_layers:
   uv=mesh.uv_layers.new(name=layer.name)
   for srcface,dstface in zip(keep,mesh.polygons):
    for si,di in zip(srcface.loop_indices,dstface.loop_indices):uv.data[di].uv=layer.data[si].uv
  for i in range(native):
   assert tuple(mesh.polygons[i].vertices)==tuple(m.polygons[i].vertices)
   assert all(tuple(mesh.uv_layers.active.data[a].uv)==tuple(m.uv_layers.active.data[b].uv) for a,b in zip(mesh.polygons[i].loop_indices,m.polygons[i].loop_indices))
  leaf.data=mesh;bpy.data.libraries.write(str(out/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(out/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(out/'actual'/f'{mode}.png')
  d.update(model_sha256=sha(out/'worker.blend'),prior_model_sha256=sha(src/'worker.blend'),neighbor_clearance=dict(removed_inferred_faces=removed,total_before=len(m.polygons),native_faces_unchanged=native,reserved_bark_samples=len(points),all_vertices_and_retained_uv_exact=True,known_source_images_and_wood_unchanged=True));write_json(out/'receipt.json',d);print('REMOVED',len(removed),'OF',len(m.polygons))
 finally:release()
if __name__=='__main__':main()

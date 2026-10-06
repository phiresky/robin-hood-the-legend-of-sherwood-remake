"""Connect private canopy clusters with inferred tapered branches behind known foliage."""
import sys,math,json,shutil,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
def main():
 tree=int(sys.argv[sys.argv.index('--')+1]);assert tree in (12,14);old,new=(4,5) if tree==12 else (2,3);src=B/f'tree{tree}-crownfragment-v{old}';out=B/f'tree{tree}-crownfragment-v{new}';assert shutil.disk_usage(ROOT).free>25*1024**3;out.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];stems=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==f'croisement03-tree-{tree}' and not o.get('inferred_branch')];assert len(stems)==(3 if tree==12 else 2);images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};records=[];x0=946 if tree==12 else 1084;base=-400 if tree==12 else -310
  for j,(xx,yy) in enumerate(((13,20),(68,22),(28,40),(79,49),(12,66),(59,72),(37,92),(78,99))):
   depth=base-42*math.sin(xx*.127+yy*.079)-17*math.cos(xx*.263-yy*.117);leaf=Vector((x0+xx,depth,(-depth*SIN-yy)/COS));end=leaf-RAY*35;stem=min(stems,key=lambda o:abs(sum(v.co.x for v in o.data.vertices)/len(o.data.vertices)-end.x));levels={}
   for v in stem.data.vertices:levels.setdefault(round(v.co.z,3),[]).append(v.co.copy())
   z=min(levels,key=lambda z:abs(z-(end.z-20)));start=sum(levels[z],Vector())/len(levels[z]);mid=start.lerp(end,.6)+Vector((0,0,5));axis=(end-start).normalized();u=axis.cross(Vector((0,0,1))).normalized();v=axis.cross(u);vertices=[];faces=[]
   for c,r in ((start,1.65),(mid,.9),(end,.18)):
    for k in range(8):vertices.append(tuple(c+(u*math.cos(k*math.tau/8)+v*math.sin(k*math.tau/8))*r))
   faces.append(tuple(reversed(range(8))))
   for ring in range(2):
    for k in range(8):faces.append((ring*8+k,ring*8+(k+1)%8,(ring+1)*8+(k+1)%8,(ring+1)*8+k))
   faces.append(tuple(range(16,24)));mesh=bpy.data.meshes.new('Inferred lower crown support');mesh.from_pydata(vertices,[],faces);mesh.update();mesh.materials.append(stem.data.materials[-1]);uv=mesh.uv_layers.new(name='UVMap')
   for f in mesh.polygons:
    f.use_smooth=len(f.vertices)==4
    for li in f.loop_indices:uv.data[li].uv=(-1,-1)
   o=bpy.data.objects.new(f'Inferred cluster support {j}',mesh);scene.collection.objects.link(o);o['asset_group']=f'croisement03-tree-{tree}';o['inferred_branch']=True;o['source_node']=stem['source_node'];records.append(dict(start=list(start),end=list(end),source_projection=[x0+xx,yy],inferred=True))
  assert images=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};bpy.data.libraries.write(str(out/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(out/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(out/'actual'/f'{mode}.png')
  write_json(out/'receipt.json',dict(status='Private morphology revision; source/contact review pending',model_sha256=sha(out/'worker.blend'),parent_model_sha256=sha(src/'worker.blend'),original_images_exact=True,unchanged_stem_and_leaf_geometry=True,added_supports=records,limits=['Branches inferred, unknown bark stays gray pending approval/fill.','No new source ownership; shared dynamic frame partition remains provisional.']))
 finally:release()
if __name__=='__main__':main()

"""Private adjacent tree context with explicit approved and unfinished components."""
import sys,math,json,shutil
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def stored_world(o):return stored_world(o.parent)@o.matrix_parent_inverse@o.matrix_basis if o.parent else o.matrix_basis.copy()
def main():
 out=B/'tree08-crown-joint-v2';assert shutil.disk_usage(ROOT).free>25*1024**3;out.mkdir(exist_ok=False);model=B/'tree08-crown-prototype-v1/worker.blend';sources={str(model):sha(model)};acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;imports=[]
  plans=[(B/'tree09-crown-prototype-v2/worker.blend',{'croisement03-tree-09','croisement03-arbre06-fragment-tree09-provisional'},10,'Root-reviewed Tree09 static geometry, user approval pending'),(B.parent/'croisement03-grouped.blend',{'croisement03-tree-07'},3,'Untouched coarse neighboring Tree07; unfinished')]
  for path,groups,count,status in plans:
   sources[str(path)]=sha(path)
   with bpy.data.libraries.load(str(path),link=False) as (a,b):b.objects=list(a.objects)
   matches=[o for o in b.objects if o.type=='MESH' and o.get('asset_group') in groups];assert len(matches)==count,(path,len(matches));matrices={o:stored_world(o) for o in matches}
   for o in matches:o.parent=None;scene.collection.objects.link(o);o.matrix_world=matrices[o];o.hide_render=False;o.hide_viewport=False
   imports.append(dict(path=str(path),count=count,status=status))
  mesh=bpy.data.meshes.new('Diagnostic woodland ground');mesh.from_pydata([(670,-460,0),(870,-460,0),(870,-160,0),(670,-160,0)],[],[(0,1,2,3)]);ground=bpy.data.objects.new('Diagnostic woodland ground',mesh);scene.collection.objects.link(ground);target=Vector((767,-290,165));views={}
  for i in range(8):
   cam=next(o for o in scene.objects if o.type=='CAMERA' and o.name==f'Tree13 view{i}');a=i*math.tau/8;direction=Vector((math.sin(a)*math.cos(math.radians(35)),-math.cos(a)*math.cos(math.radians(35)),math.sin(math.radians(35))));cam.location=target+direction*1500;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=510;views[f'view-{i}']=cam.name
  render_views(scene.name,views,out,modes=('textured',),width=384);sheet=Image.new('RGB',(1536,768),'#333333')
  for i in range(8):
   im=Image.open(out/f'view-{i}-textured.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
  sheet.save(out/'sheet.png');assert all(sha(Path(p))==v for p,v in sources.items());write_json(out/'receipt.json',dict(status='Private joint geometry diagnostic; visual review required',source_hashes=sources,imports=imports,native_view_index=0,sheet_sha256=sha(out/'sheet.png'),limits=['Diagnostic ground and surrounding ivy/foliage unfinished; full map scene not approved.','Arbre06 spatial fragment is private geometry context, no static ownership or complete runtime animation integration.','Private Tree09 remains byte unchanged; coarse Tree07 is context only.']))
 finally:release()
if __name__=='__main__':main()

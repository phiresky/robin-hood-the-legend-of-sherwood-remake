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
 out=B/'approved-hub-textures-v1/croisement03-tree-10/wood-input-v1/filled-contact-v1';assert shutil.disk_usage(ROOT).free>10*1024**3;out.mkdir(exist_ok=False);model=B/'approved-hub-textures-v1/croisement03-tree-10/wood-input-v1/source-restored-fill-v1/worker.blend';sources={str(model):sha(model)};available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024;assert available>=6*1024**3;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement03 Refinement'];bpy.context.window.scene=scene;scene.cycles.transparent_max_bounces=256;scene.render.threads_mode='FIXED';scene.render.threads=2;imports=[]
  plans=[(B/'tree11-crown-prototype-v4/worker.blend',{'croisement03-tree-11','croisement03-arbre06-fragment-tree11-provisional'},21,'User-approved Tree11 geometry; its gray inferred bark appearance remains pending'),(B/'tree09-crown-prototype-v2/worker.blend',{'croisement03-tree-09','croisement03-arbre06-fragment-tree09-provisional'},None,'User-approved Tree09 geometry; its gray inferred bark appearance remains pending')]
  for path,groups,count,status in plans:
   sources[str(path)]=sha(path)
   with bpy.data.libraries.load(str(path),link=False) as (a,b):b.objects=list(a.objects)
   matches=[o for o in b.objects if o.type=='MESH' and o.get('asset_group') in groups];assert matches and (count is None or len(matches)==count),(path,len(matches));matrices={o:stored_world(o) for o in matches}
   for o in matches:o.parent=None;scene.collection.objects.link(o);o.matrix_world=matrices[o];o.hide_render=False;o.hide_viewport=False
   imports.append(dict(path=str(path),count=count,status=status))
  mesh=bpy.data.meshes.new('Diagnostic woodland ground');mesh.from_pydata([(740,-460,0),(980,-460,0),(980,-160,0),(740,-160,0)],[],[(0,1,2,3)]);ground=bpy.data.objects.new('Diagnostic woodland ground',mesh);scene.collection.objects.link(ground);target=Vector((853,-305,165));views={}
  for i in (0,2):
   cam=next(o for o in scene.objects if o.type=='CAMERA' and o.name==f'Tree13 view{i}');a=i*math.tau/8;direction=Vector((math.sin(a)*math.cos(math.radians(35)),-math.cos(a)*math.cos(math.radians(35)),math.sin(math.radians(35))));cam.location=target+direction*1500;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=510;views[f'view-{i}']=cam.name
  render_views(scene.name,views,out,modes=('textured',),width=384);sheet=Image.new('RGB',(768,384),'#333333')
  for column,i in enumerate((0,2)):
   im=Image.open(out/f'view-{i}-textured.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(column*384,0))
  sheet.save(out/'sheet.png');assert all(sha(Path(p))==v for p,v in sources.items());write_json(out/'receipt.json',dict(status='Private joint geometry diagnostic; visual review required',source_hashes=sources,imports=imports,native_view_index=0,sheet_sha256=sha(out/'sheet.png'),limits=['Diagnostic ground and surrounding ivy/foliage unfinished; full map scene not approved.','Arbre06 spatial fragment is private geometry context, no static ownership or complete runtime animation integration.','Approved Tree09/11 remain byte unchanged; their gray unknown wood is contextual and excluded from Tree10 appearance scope.']))
 finally:release()
if __name__=='__main__':main()

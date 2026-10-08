"""Private adjacent tree context with explicit approved and unfinished components."""
import sys,math,json,shutil
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def stored_world(o):return stored_world(o.parent)@o.matrix_parent_inverse@o.matrix_basis if o.parent else o.matrix_basis.copy()
def main():
 out=B/'approved-hub-textures-v1/static-pair-export-v1/joint-contact-v2';assert shutil.disk_usage(ROOT).free>10*1024**3;out.mkdir(exist_ok=False);model=B/'tree12-static-leaf-crown-v3/worker.blend';sources={str(model):sha(model)};available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024;assert available>=6*1024**3;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement03 Refinement'];bpy.context.window.scene=scene;scene.cycles.transparent_max_bounces=256;scene.render.threads_mode='FIXED';scene.render.threads=2;imports=[]
  for obj in list(scene.objects):
   if obj.type=='MESH':bpy.data.objects.remove(obj,do_unlink=True)
  for number in (12,14):
   case=B/f'approved-hub-textures-v1/static-pair-export-v1/tree{number}';report=json.loads((case/'report.json').read_text());path=case/'model.glb';assert sha(path)==report['model_sha256'];assert json.loads((case/'native-proof.json').read_text())['status']=='PASS';sources[str(path)]=sha(path);before=set(scene.objects);bpy.ops.import_scene.gltf(filepath=str(path));new=set(scene.objects)-before;shift=Matrix.Translation(Vector(report['export']['placement_origin_scene']))
   for obj in new:
    if obj.parent is None:obj.matrix_world=shift@obj.matrix_world
   tint_links=0
   for obj in new:
    if obj.type!='MESH':continue
    for material in obj.data.materials:
     if not material or not material.get('crown_source_role'):continue
     for link in list(material.node_tree.links):
      if link.from_node.type=='VERTEX_COLOR':
       socket=link.to_socket;material.node_tree.links.remove(link);socket.default_value=(1,1,1,1) if socket.type=='RGBA' else 1;tint_links+=1
   assert tint_links>0,'Expected imported glTF ownership-color tint to neutralize only in diagnostic'
   imports.append(dict(path=str(path),count=sum(o.type=='MESH' for o in new),status='Exact approved model export with both crown roles',diagnostic_ownership_tint_links_neutralized=tint_links))
  bpy.context.view_layer.update()
  plans=[(B/'tree13-approved-wood-texture-v2/source-restored-fill-v1/worker.blend',{'croisement03-tree-13','croisement03-local75-provisional'},4,'User geometry and appearance approved V14'),(B/'texture-round1/croisement03-fern-35/experiment/baked-preserved-v1/worker.blend',{'croisement03-fern-35'},1,'User-approved fern35'),(B/'texture-round1/croisement03-fern-76/experiment/baked-preserved-v1/worker.blend',{'croisement03-fern-76'},1,'User-approved fern76')]
  for path,groups,count,status in plans:
   sources[str(path)]=sha(path)
   with bpy.data.libraries.load(str(path),link=False) as (a,b):b.objects=list(a.objects)
   matches=[o for o in b.objects if o.type=='MESH' and o.get('asset_group') in groups];assert len(matches)==count,(path,len(matches));matrices={o:stored_world(o) for o in matches}
   for o in matches:o.parent=None;scene.collection.objects.link(o);o.matrix_world=matrices[o];o.hide_render=False;o.hide_viewport=False
   imports.append(dict(path=str(path),count=count,status=status))
  mesh=bpy.data.meshes.new('Diagnostic woodland ground');mesh.from_pydata([(900,-450,0),(1210,-450,0),(1210,-100,0),(900,-100,0)],[],[(0,1,2,3)]);ground=bpy.data.objects.new('Diagnostic woodland ground',mesh);scene.collection.objects.link(ground);target=Vector((1030,-275,165));views={}
  for i in (0,2):
   cam=next(o for o in scene.objects if o.type=='CAMERA' and o.name==f'Tree13 view{i}');a=i*math.tau/8;direction=Vector((math.sin(a)*math.cos(math.radians(35)),-math.cos(a)*math.cos(math.radians(35)),math.sin(math.radians(35))));cam.location=target+direction*1500;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=510;views[f'view-{i}']=cam.name
  render_views(scene.name,views,out,modes=('textured',),width=384);sheet=Image.new('RGB',(768,384),'#333333')
  for column,i in enumerate((0,2)):
   im=Image.open(out/f'view-{i}-textured.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(column*384,0))
  sheet.save(out/'sheet.png');assert all(sha(Path(p))==v for p,v in sources.items());write_json(out/'receipt.json',dict(status='Private joint geometry diagnostic; visual review required',source_hashes=sources,imports=imports,native_view_index=0,sheet_sha256=sha(out/'sheet.png'),limits=['Diagnostic ground and surrounding ivy/foliage unfinished; full map scene not approved.','Arbre06 spatial fragment is private geometry context, no static ownership or complete runtime animation integration.','Frozen13 and approved ferns are byte unchanged. Both static additions are explicitly approved. Imported physical material contact diagnostic; production runtime views are separate.']))
 finally:release()
if __name__=='__main__':main()

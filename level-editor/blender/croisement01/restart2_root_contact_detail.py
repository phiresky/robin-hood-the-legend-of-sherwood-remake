"""Root detail against archived terrain, preserving frozen camera directions."""
import argparse,json,sys
from pathlib import Path
import bpy
from mathutils import Matrix,Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from evidence_io import sha

def main():
 p=argparse.ArgumentParser();p.add_argument('workspace',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.workspace.resolve();out=w/'inspection/root-contact-detail'
 if out.exists():raise FileExistsError(out)
 acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text())
 scene=bpy.data.scenes.new('Archived terrain contact diagnostic');scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256
 scene.render.resolution_x=scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG'
 scene.world=bpy.data.worlds.new('Contact ambient');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs['Color'].default_value=(.3,.3,.3,1)
 scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
 mat=bpy.data.materials.new('Neutral archived support');mat.diffuse_color=(.3,.3,.3,1);mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.3,.3,.3,1)
 nodes={'ground'}|{f'building-{i:03d}' for i in list(range(10))+list(range(76,81))}
 for obj in list(bpy.data.collections[cfg['collection_name']].all_objects):
  target=obj.get('asset_group')==cfg['asset_id'];terrain=obj.get('source_node') in nodes
  if obj.type!='MESH' or not(target or terrain):continue
  copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False
  if terrain:
   copy.data=obj.data.copy();copy.data.materials.clear();copy.data.materials.append(mat)
   for face in copy.data.polygons:face.material_index=0
  scene.collection.objects.link(copy)
 sun_data=bpy.data.lights.new('Contact light','SUN');sun_data.energy=2;sun_data.angle=.08
 sun=bpy.data.objects.new('Contact light',sun_data);scene.collection.objects.link(sun);sun.rotation_euler=Vector((-.6,-.4,-.7)).to_track_quat('-Z','Y').to_euler()
 data=bpy.data.cameras.new('Frozen contact camera');data.type='ORTHO';data.clip_end=20000;camera=bpy.data.objects.new('Frozen contact camera',data);scene.collection.objects.link(camera);scene.camera=camera
 packet=json.loads((w/'modified/views.json').read_text());out.mkdir();images=[]
 roots=[o.matrix_world@vert.co for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id'] and o.get('source_node','').startswith('building-') for vert in o.data.vertices]
 low=min(p.z for p in roots);basal=[p for p in roots if p.z<low+5];center=sum(basal,Vector())/len(basal);center.z=low+45
 for i in [0,1,3,5]:
  v=packet['views'][i];camera.matrix_world=Matrix(v['camera_matrix_world']);data.ortho_scale=160;camera.location=center+camera.matrix_world.to_3x3()@Vector((0,0,5000));scene.render.filepath=str(out/f'view-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name);images.append(Image.open(scene.render.filepath).convert('RGB'))
 sheet=Image.new('RGB',(768,768))
 for i,im in enumerate(images):sheet.paste(im,((i%2)*384,(i//2)*384))
 sheet.save(out/'sheet.png');(out/'evidence.json').write_text(json.dumps(dict(status='diagnostic; archived terrain is not refined or approved',model_sha256=sha(w/'model.blend'),views_sha256=sha(w/'modified/views.json'),sheet_sha256=sha(out/'sheet.png'),camera_indices=[0,1,3,5],camera_direction_preserved=True,root_center=list(center),detail_scale=160,terrain_material_override='neutral diffuse; saved asset materials preserved'),indent=2)+'\n');release()
if __name__=='__main__':main()

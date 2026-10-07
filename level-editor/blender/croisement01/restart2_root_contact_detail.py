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
 p=argparse.ArgumentParser();p.add_argument('workspace',type=Path);p.add_argument('--overlay-worker',type=Path);p.add_argument('--overlay-asset');p.add_argument('--color-terrain-owner',action='store_true');p.add_argument('--context-node',action='append',default=[]);p.add_argument('--scale',type=float,default=160);p.add_argument('--wood-node',action='append',default=[]);p.add_argument('--output-name',default='root-contact-detail');a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.workspace.resolve();out=w/'inspection'/a.output_name
 if Path(a.output_name).name!=a.output_name or a.scale<=0:raise ValueError('Invalid detail output or scale')
 if out.exists():raise FileExistsError(out)
 acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text())
 review_objects=list(bpy.data.collections[cfg['collection_name']].all_objects);target_asset=cfg['asset_id'];camera_worker=w;overlay_proof=None
 if a.overlay_worker:
  if not a.overlay_asset:raise ValueError('Overlay requires exact asset ID')
  overlay=a.overlay_worker.resolve();model=overlay/'model.blend'
  with bpy.data.libraries.load(str(model),link=False) as (src,dst):dst.objects=list(src.objects)
  imported=[o for o in dst.objects if o is not None]
  for obj in imported:bpy.context.scene.collection.objects.link(obj)
  bpy.context.view_layer.update();targets=[o for o in imported if o.type=='MESH' and o.get('asset_group')==a.overlay_asset];assert targets
  reference_path=ROOT/'level-editor/work/croisement01-refinement/restart2/bank-neighbor-transform-reference-v2.json';reference=next(row for row in json.loads(reference_path.read_text())['sources'] if row['model_sha256']==sha(model))
  for obj in targets:
   expected=[row for row in reference['objects'] if row['source_node']==obj.get('source_node')]
   if not expected:
    assert 'foliage' in obj.get('source_node','') or obj.get('projection_component')=='crown';continue
   assert len(expected)==1
   assert max(abs(obj.matrix_world[i][j]-expected[0]['matrix_world'][i][j]) for i in range(4) for j in range(4))<1e-5
  review_objects+=targets;target_asset=a.overlay_asset;camera_worker=overlay;overlay_proof=dict(model_sha256=sha(model),transform_reference_sha256=sha(reference_path),asset_id=target_asset)
 scene=bpy.data.scenes.new('Archived terrain contact diagnostic');scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256
 scene.render.resolution_x=scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG'
 scene.world=bpy.data.worlds.new('Contact ambient');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs['Color'].default_value=(.3,.3,.3,1)
 scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
 mat=bpy.data.materials.new('Neutral archived support');mat.diffuse_color=(.3,.3,.3,1);mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.3,.3,.3,1)
 nodes={'ground'}|{f'building-{i:03d}' for i in list(range(10))+list(range(76,81))}|set(a.context_node)
 owner_colors={};owner_materials={}
 if a.color_terrain_owner:
  import colorsys
  for i,node in enumerate(sorted(nodes)):
   color=(*colorsys.hsv_to_rgb(i/len(nodes),.65,.8),1);m=mat.copy();m.name='Diagnostic '+node;m.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=color;owner_colors[node]=list(color);owner_materials[node]=m
 for obj in review_objects:
  target=obj.get('asset_group')==target_asset;terrain=obj.get('source_node') in nodes
  if obj.type!='MESH' or not(target or terrain):continue
  copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False
  if terrain:
   copy.data=obj.data.copy();copy.data.materials.clear();copy.data.materials.append(owner_materials.get(obj.get('source_node'),mat))
   for face in copy.data.polygons:face.material_index=0
  scene.collection.objects.link(copy)
 sun_data=bpy.data.lights.new('Contact light','SUN');sun_data.energy=2;sun_data.angle=.08
 sun=bpy.data.objects.new('Contact light',sun_data);scene.collection.objects.link(sun);sun.rotation_euler=Vector((-.6,-.4,-.7)).to_track_quat('-Z','Y').to_euler()
 data=bpy.data.cameras.new('Frozen contact camera');data.type='ORTHO';data.clip_end=20000;camera=bpy.data.objects.new('Frozen contact camera',data);scene.collection.objects.link(camera);scene.camera=camera
 packet=json.loads((camera_worker/'modified/views.json').read_text());out.mkdir();images=[]
 roots=[o.matrix_world@vert.co for o in review_objects if o.type=='MESH' and o.get('asset_group')==target_asset and (o.get('source_node','').startswith('building-') or o.get('source_node') in a.wood_node) for vert in o.data.vertices]
 if not roots:raise ValueError('No explicit woody target node found')
 low=min(p.z for p in roots);basal=[p for p in roots if p.z<low+5];center=sum(basal,Vector())/len(basal);center.z=low+45
 for i in [0,1,3,5]:
  v=packet['views'][i];camera.matrix_world=Matrix(v['camera_matrix_world']);data.ortho_scale=a.scale;camera.location=center+camera.matrix_world.to_3x3()@Vector((0,0,5000));scene.render.filepath=str(out/f'view-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name);images.append(Image.open(scene.render.filepath).convert('RGB'))
 sheet=Image.new('RGB',(768,768))
 for i,im in enumerate(images):sheet.paste(im,((i%2)*384,(i//2)*384))
 sheet.save(out/'sheet.png');(out/'evidence.json').write_text(json.dumps(dict(status='diagnostic; context is not approved final terrain',overlay=overlay_proof,terrain_owner_colors=owner_colors,additional_context_nodes=a.context_node,additional_wood_nodes=a.wood_node,model_sha256=sha(w/'model.blend'),views_sha256=sha(camera_worker/'modified/views.json'),sheet_sha256=sha(out/'sheet.png'),camera_indices=[0,1,3,5],camera_direction_preserved=True,root_center=list(center),detail_scale=a.scale,terrain_material_override='neutral diffuse; saved asset materials preserved'),indent=2)+'\n');release()
if __name__=='__main__':main()

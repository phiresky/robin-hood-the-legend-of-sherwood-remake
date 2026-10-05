"""Review saved branch and grass materials together against archival terrain."""
import json
import argparse
import math
import sys
from pathlib import Path
import bpy
from mathutils import Matrix,Vector
from PIL import Image,ImageDraw

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from evidence_io import sha


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--grass-revision',type=int,default=9);parser.add_argument('--joint-revision',type=int,default=5);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    out=ROOT/'level-editor/work/croisement01-refinement'
    worker=out/f'restart2/grass75-volume-v{args.grass_revision}/assets/croisement01-grass-75'
    branch=out/'restart2/branch-source-fit-v3/branch-round-10/assets/croisement01-east-fallen-branch'
    dest=out/f'restart2/branch-grass-joint-v{args.joint_revision}';dest.mkdir(exist_ok=False)
    acquire();model_sha=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    scene=bpy.data.scenes.new('Branch and Grass Joint');scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256
    scene.world=bpy.data.worlds.new('Joint ambient');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs['Color'].default_value=(.3,.3,.3,1)
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.image_settings.file_format='PNG';scene.render.resolution_percentage=100
    support=bpy.data.materials.new('Neutral archival support');support.use_nodes=True;support.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.3,.3,.3,1)
    terrain_nodes={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
    terrain=[];targets=[]
    for obj in list(bpy.data.collections['Croisement01 Working'].all_objects):
        if obj.type!='MESH':continue
        target=obj.get('asset_group') in {'croisement01-grass-75','croisement01-east-fallen-branch'}
        ground=obj.get('source_node') in terrain_nodes
        if not(target or ground):continue
        copied=obj.copy();copied.parent=None;copied.matrix_world=obj.matrix_world.copy();copied.hide_render=False
        if ground:
            copied.data=obj.data.copy();copied.data.materials.clear();copied.data.materials.append(support)
            for poly in copied.data.polygons:poly.material_index=0
            terrain.append(copied)
        else:targets.append(copied)
        scene.collection.objects.link(copied)
    if len(targets)!=2:raise ValueError(f'Expected branch and grass, got {len(targets)}')
    light=bpy.data.lights.new('Joint sun','SUN');light.energy=2;light.angle=.08;sun=bpy.data.objects.new('Joint sun',light);scene.collection.objects.link(sun);sun.rotation_euler=Vector((-.6,-.4,-.7)).to_track_quat('-Z','Y').to_euler()
    data=bpy.data.cameras.new('Frozen branch camera');data.type='ORTHO';data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
    packet=json.loads((branch/'modified/views.json').read_text());images=[]
    scene.render.resolution_x=scene.render.resolution_y=384
    for index,view in enumerate(packet['views']):
        camera.matrix_world=Matrix(view['camera_matrix_world']);data.ortho_scale=view['ortho_scale'];scene.render.filepath=str(dest/f'view-{index}.png');bpy.ops.render.render(write_still=True,scene=scene.name);images.append(Image.open(scene.render.filepath).convert('RGB'))
    sheet=Image.new('RGB',(1536,768))
    for index,image in enumerate(images):sheet.paste(image,((index%4)*384,(index//4)*384))
    sheet.save(dest/'sheet.png')
    for obj in terrain:obj.hide_render=True
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));box=[1020,278,1235,375];left,top,right,bottom=box
    center=Vector(((left+right)/2,-(top+bottom)/2/sine,0));camera.location=center+Vector((0,-cosine,sine))*5000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();data.sensor_fit='HORIZONTAL';data.ortho_scale=right-left
    scene.render.resolution_x=right-left;scene.render.resolution_y=bottom-top;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';scene.render.filepath=str(dest/'native-actual.png');bpy.ops.render.render(write_still=True,scene=scene.name)
    source=Image.open(out/'baseline/covered.png').convert('RGBA').crop(box);actual=Image.open(dest/'native-actual.png').convert('RGBA');board=Image.new('RGB',(source.width*12,source.height*4+24),'#444444');draw=ImageDraw.Draw(board)
    for i,(label,image) in enumerate([('Native source',source),('Saved branch and grass',actual),('Overlay',Image.alpha_composite(source,actual))]):
        draw.text((i*source.width*4+4,4),label,fill='white');image=image.resize((source.width*4,source.height*4),Image.Resampling.NEAREST);board.paste(image,(i*source.width*4,24),image)
    board.save(dest/'native-comparison.png')
    (dest/'evidence.json').write_text(json.dumps(dict(status='joint visual inspection pending',worker_model_sha256=model_sha,branch_model_sha256=sha(branch/'model.blend'),sheet_sha256=sha(dest/'sheet.png'),native_comparison_sha256=sha(dest/'native-comparison.png'),limitations=['Archival bank remains provisional.','Grass and branch ownership is not yet integrated into canonical scene.']),indent=2)+'\n')
    if sha(worker/'model.blend')!=model_sha:raise ValueError('Review changed saved model')


if __name__=='__main__':main()

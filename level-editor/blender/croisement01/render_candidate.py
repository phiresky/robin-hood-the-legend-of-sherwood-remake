"""Review actual saved materials, fixed eight cameras and native source framing."""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from render_multiview_asset import render
from evidence_io import sha


def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);parser.add_argument('--native-context-node',action='append',default=[])
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    workspace=args.workspace.resolve();acquire()
    model_hash=sha(workspace/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'))
    config=json.loads((workspace/'workspace.json').read_text())
    scene=bpy.data.scenes[config['scene_name']]
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256
    scene.world=bpy.data.worlds.new('Actual material review');scene.world.color=(.1,.1,.1)
    output=workspace/'inspection/actual-materials'
    if output.exists():raise FileExistsError(output)
    packet=json.loads((workspace/'modified/views.json').read_text())
    for view in packet['views']:
        view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]}
    manifest=workspace/'inspection/actual-camera-manifest.json'
    manifest.write_text(json.dumps(packet,indent=2)+'\n')
    render(manifest,output,width=256)
    images=[Image.open(output/f'view-{i}-textured.png').convert('RGB') for i in range(8)]
    width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
    for i,image in enumerate(images):sheet.paste(image,((i%4)*width,(i//4)*height))
    sheet.save(output/'sheet.png')
    objects=[o for o in bpy.data.collections[config['collection_name']].all_objects
             if o.type=='MESH' and (o.get('asset_group')==config['asset_id'] or o.get('source_node') in args.native_context_node)]
    native=bpy.data.scenes.new('Native source appearance audit')
    points=[]
    for obj in objects:
        copied=obj.copy();copied.parent=None;copied.matrix_world=obj.matrix_world.copy()
        copied.hide_render=False;native.collection.objects.link(copied)
        points.extend(obj.matrix_world@v.co for v in obj.data.vertices)
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    source=Image.open(config['source_path']).convert('RGBA')
    xs=[p.x for p in points];ys=[-p.y*sine-p.z*cosine for p in points]
    box=[max(0,math.floor(min(xs)-16)),max(0,math.floor(min(ys)-16)),
         min(source.width,math.ceil(max(xs)+16)),min(source.height,math.ceil(max(ys)+16))]
    left,top,right,bottom=box;width,height=right-left,bottom-top
    target=Vector(((left+right)/2,-(top+bottom)/2/sine,0))
    camera_data=bpy.data.cameras.new('Native source camera');camera_data.type='ORTHO'
    camera_data.sensor_fit='HORIZONTAL';camera_data.ortho_scale=width;camera_data.clip_end=20000
    camera=bpy.data.objects.new(camera_data.name,camera_data);native.collection.objects.link(camera)
    camera.location=target+Vector((0,-cosine,sine))*5000
    camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();native.camera=camera
    native.render.engine='CYCLES';native.cycles.samples=16;native.cycles.transparent_max_bounces=256
    native.render.resolution_x=width;native.render.resolution_y=height;native.render.resolution_percentage=100
    native.render.film_transparent=True;native.render.image_settings.file_format='PNG';native.render.image_settings.color_mode='RGBA'
    native.view_settings.view_transform='Standard';native.view_settings.look='None'
    comparison=workspace/'inspection/native-source';comparison.mkdir()
    native.render.filepath=str(comparison/'actual.png');bpy.ops.render.render(write_still=True,scene=native.name)
    original=source.crop(box);actual=Image.open(comparison/'actual.png').convert('RGBA')
    original.save(comparison/'source.png')
    board=Image.new('RGB',(width*4*3,height*4+28),'#444444');draw=ImageDraw.Draw(board)
    for i,(label,im) in enumerate([('Native source',original),('Saved material',actual),('Overlay',Image.alpha_composite(original,actual))]):
        draw.text((i*width*4+4,5),label,fill='white')
        enlarged=im.resize((width*4,height*4),Image.Resampling.NEAREST)
        board.paste(enlarged,(i*width*4,28),enlarged)
    board.save(comparison/'comparison.png')
    assert sha(workspace/'model.blend')==model_hash
    (output/'evidence.json').write_text(json.dumps(dict(model_sha256=model_hash,
        actual_sheet_sha256=sha(output/'sheet.png'),native_comparison_sha256=sha(comparison/'comparison.png'),
        source_crop=box,native_context_nodes=args.native_context_node,status='rendered; visual judgment and independent coverage audit pending'),indent=2)+'\n')
    print(output/'sheet.png')


if __name__=='__main__':main()

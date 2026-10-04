"""Read-only native-camera audit of mixed scenery masks against existing owners."""
import argparse,json,sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY

def main(directory=None,model_override=None):
    directory=directory or OUT/'mixed-wood-audit';directory.mkdir(parents=True,exist_ok=True)
    native=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    for index,asset,worker in [(76,'croisement02-southwest-path-wattle-fence',scenery_workspace('croisement02-southwest-path-wattle-fence')),(93,'croisement02-tree-35',tree_workspace(35))]:
        if model_override and index!=93:continue
        model=Path(model_override) if model_override else worker/'model.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')==asset]
        if not objects:raise ValueError('No matching existing geometry')
        record=native['masks'][index];x,y=record['box_top_left'];w,h=record['box_size'];left=x-20;top=y-20;width=w+40;height=h+40
        scene=bpy.data.scenes.new('Mixed wood audit');copies=[]
        for obj in objects:
            copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy);copies.append(copy)
        target=Vector((left+width/2,-(top+height/2)/SIN,0));data=bpy.data.cameras.new('Native camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000
        camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
        scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64;scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.filepath=str(directory/f'{index}-existing-render.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop((left,top,left+width,top+height));render=Image.open(scene.render.filepath).convert('RGBA');overlay=source.copy();overlay.alpha_composite(render);sheet=Image.new('RGBA',(width*3,height),(100,100,100,255));sheet.paste(source,(0,0));sheet.alpha_composite(render,(width,0));sheet.paste(overlay,(width*2,0));sheet.resize((width*9,height*3)).save(directory/f'{index}-existing-comparison.png')
        if sha(model)!=digest:raise ValueError('Existing owner file changed')
        write_json(directory/f'{index}-existing-evidence.json',dict(native_mask=index,asset=asset,worker=str(worker),inspected_model=str(model),model_sha256=digest,source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),render_sha256=sha(Path(scene.render.filepath)),comparison_sha256=sha(directory/f'{index}-existing-comparison.png'),crop=[left,top,left+width,top+height],mesh_count=len(objects),baseline_unchanged=True))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--directory',type=Path);parser.add_argument('--model',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    acquire()
    try:main(args.directory,args.model)
    finally:release()

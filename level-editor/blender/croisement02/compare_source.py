"""Render saved scenery from the exact map camera beside its source artwork."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from evidence_io import sha,write_json


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--assets',nargs='+',required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    for slug in args.assets:
        w=OUT/('scenery-round-2' if slug in ('north-kindling-bundle','south-field-wattle-fence') else 'scenery-round-1')/'assets'/('croisement02-'+slug);acquire();model_hash=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
        scene=bpy.data.scenes.new('Exact scenery source comparison');objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name];points=[]
        for o in objects:
            obj=o.copy();obj.parent=None;obj.matrix_world=o.matrix_world.copy();obj.hide_render=False;scene.collection.objects.link(obj);points.extend(o.matrix_world@v.co for v in o.data.vertices)
        xy=np.array([(p.x,-p.y*SIN-p.z*COS) for p in points]);left,top=np.floor(xy.min(axis=0)-8).astype(int);right,bottom=np.ceil(xy.max(axis=0)+8).astype(int);left=max(0,left);top=max(0,top);right=min(1792,right);bottom=min(1152,bottom);width,height=right-left,bottom-top
        target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Map camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000
        camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
        scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64;scene.render.resolution_x=int(width);scene.render.resolution_y=int(height);scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
        out=w/'inspection/source-comparison';out.mkdir(exist_ok=True);scene.render.filepath=str(out/'render.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop((left,top,right,bottom));render=Image.open(out/'render.png').convert('RGBA');composite=Image.alpha_composite(source,render)
        scale=max(1,min(4,1100//width));sheet=Image.new('RGB',(width*scale,height*scale*3+72),'#ddd');draw=ImageDraw.Draw(sheet)
        for n,(label,im) in enumerate([('Original source',source),('Saved geometry from map camera',render),('Geometry over source',composite)]):
            draw.text((4,n*(height*scale+24)+4),label,fill='black');sheet.paste(im.resize((width*scale,height*scale),Image.Resampling.NEAREST),(0,n*(height*scale+24)+24),im.resize((width*scale,height*scale)).getchannel('A'))
        sheet.save(out/'comparison.png')
        if sha(w/'model.blend')!=model_hash:raise ValueError('Model changed during source comparison')
        write_json(out/'report.json',dict(model_sha256=sha(w/'model.blend'),comparison_sha256=sha(out/'comparison.png'),crop=[int(v) for v in (left,top,right,bottom)]));release();print('COMPARED',slug,flush=True)

if __name__=='__main__':main()

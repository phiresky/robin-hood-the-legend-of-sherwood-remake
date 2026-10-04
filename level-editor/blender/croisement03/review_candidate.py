"""Inspect a saved worker's actual materials and exact native-camera coverage."""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from render_multiview_asset import render
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))

def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);parser.add_argument('--mask',type=int,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    w=args.workspace.resolve();acquire();digest=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
    scene=bpy.data.scenes['Croisement03 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=256
    scene.world=bpy.data.worlds.new('Croisement03 neutral actual-material inspection');scene.world.color=(.1,.1,.1)
    actual=w/'inspection/actual-materials'
    if actual.exists():raise FileExistsError(actual)
    packet=json.loads((w/'modified/views.json').read_text())
    for v in packet['views']:v['crop']=dict(width=packet['tile_size'][0],height=packet['tile_size'][1])
    manifest=w/'inspection/actual-camera-manifest.json';write_json(manifest,packet);render(manifest,actual,width=256)
    images=[Image.open(actual/f'view-{i}-textured.png').convert('RGB') for i in range(8)];tw,th=images[0].size;sheet=Image.new('RGB',(tw*4,th*2))
    for i,im in enumerate(images):sheet.paste(im,((i%4)*tw,(i//4)*th))
    sheet.save(actual/'sheet.png');write_json(actual/'evidence.json',dict(model_sha256=digest,sheet_sha256=sha(actual/'sheet.png'),render_config=dict(engine='CYCLES',samples=4,transparent_max_bounces=256)))
    objects=[o for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name]
    native=bpy.data.scenes.new('Croisement03 exact native camera');points=[]
    for o in objects:
        obj=o.copy();obj.parent=None;obj.matrix_world=o.matrix_world.copy();obj.hide_render=False;native.collection.objects.link(obj);points.extend(o.matrix_world@v.co for v in o.data.vertices)
    source=Image.open(OUT/'baseline/covered.png').convert('RGBA');sw,sh=source.size
    xy=np.array([(p.x,-p.y*SIN-p.z*COS) for p in points]);lo=np.floor(xy.min(axis=0)-12).astype(int);hi=np.ceil(xy.max(axis=0)+12).astype(int)
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());mask=level['masks'][args.mask];x,y=mask['box_top_left'];mw,mh=mask['box_size'];lo=np.minimum(lo,[x-12,y-12]);hi=np.maximum(hi,[x+mw+12,y+mh+12]);left,top=np.maximum(lo,[0,0]);right,bottom=np.minimum(hi,[sw,sh]);width,height=int(right-left),int(bottom-top)
    target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Exact native camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data);native.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();native.camera=camera
    native.render.engine='CYCLES';native.cycles.samples=8;native.cycles.transparent_max_bounces=256;native.cycles.use_denoising=False;native.render.resolution_x=width;native.render.resolution_y=height;native.render.resolution_percentage=100;native.render.film_transparent=True;native.render.image_settings.file_format='PNG';native.render.image_settings.color_mode='RGBA';native.view_settings.view_transform='Standard';native.view_settings.look='None'
    out=w/'inspection/source-comparison';out.mkdir(exist_ok=False);native.render.filepath=str(out/'render.png');bpy.ops.render.render(write_still=True,scene=native.name)
    crop=source.crop((left,top,right,bottom));rendered=Image.open(out/'render.png').convert('RGBA');composite=Image.alpha_composite(crop,rendered)
    expected=Image.new('L',(sw,sh));expected.paste(Image.open(OUT/f'baseline/masks/{args.mask:06}.png'),(x,y));expected=np.asarray(expected)[top:bottom,left:right]>127;hit=np.asarray(rendered)[:,:,3]>127;missing=expected&~hit;extra=hit&~expected;diff=np.asarray(crop).copy();diff[missing]=[255,40,40,255];diff[extra]=[0,220,255,255]
    scale=max(1,min(5,1100//width));comparison=Image.new('RGB',(width*scale,height*scale*4+96),'#ddd');draw=ImageDraw.Draw(comparison)
    for n,(label,im) in enumerate([('Native source',crop),('Saved actual geometry',rendered),('Geometry over source',composite),('Red missing native mask; cyan outside mask',Image.fromarray(diff))]):
        draw.text((4,n*(height*scale+24)+4),label,fill='black');expanded=im.resize((width*scale,height*scale),Image.Resampling.NEAREST);comparison.paste(expanded,(0,n*(height*scale+24)+24),expanded.getchannel('A'))
    comparison.save(out/'comparison.png');write_json(out/'report.json',dict(model_sha256=digest,comparison_sha256=sha(out/'comparison.png'),native_mask=args.mask,crop=[int(v) for v in (left,top,right,bottom)],expected_pixels=int(expected.sum()),missing_pixels=int(missing.sum()),extra_pixels=int(extra.sum()),iou=float(np.count_nonzero(expected&hit)/np.count_nonzero(expected|hit)),status='measurement; visual interpretation required'))
    if sha(w/'model.blend')!=digest:raise ValueError('Saved model changed during inspection')
    release();print(out/'comparison.png')
if __name__=='__main__':main()

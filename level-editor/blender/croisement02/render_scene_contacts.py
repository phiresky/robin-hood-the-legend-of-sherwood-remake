"""Render labeled source/contact views of a frozen private full-scene assembly."""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from evidence_io import sha,write_json
from review_bank_candidate import camera
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
REGIONS=[('Fern119 / tree23',(450,0,750,170),44),('North shrubs65/66',(990,40,1370,330),44),('Northeast root bank',(1300,85,1550,330),0),('Northwest rock/shrubs',(0,220,480,520),0),('East rail fences',(1450,570,1792,940),0),('Southwest logs',(0,930,390,1152),0),('Southwest grass116',(0,865,240,1100),0)]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('stage',type=Path);parser.add_argument('--diagnostics-only',action='store_true');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    stage=args.stage.resolve();output=stage/'contacts'
    if output.exists() and not args.diagnostics_only:raise FileExistsError(output)
    output.mkdir(exist_ok=args.diagnostics_only);evidence=json.loads((stage/'assembly.json').read_text())
    if sha(stage/'scene.blend')!=evidence['model_sha256']:raise ValueError('Frozen scene changed')
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(stage/'scene.blend'));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        if not args.diagnostics_only:
            camera(scene,Vector((896,-576/SIN,0)),RAY,1792,1152,1792)
            scene.render.filepath=str(output/'source-camera.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        source=Image.open(output/'source-camera.png').convert('RGBA');marked=source.convert('RGB');draw=ImageDraw.Draw(marked)
        sheet=Image.new('RGB',(1440,math.ceil(len(REGIONS)/3)*410),'#333');sd=ImageDraw.Draw(sheet);records=[]
        for i,(label,box,z) in enumerate(REGIONS):
            color=['#ff6666','#66ffff','#ffff66','#ff66ff','#66ff66','#ffbb55','#aaaaff'][i]
            draw.rectangle(box,outline=color,width=3);draw.text((box[0]+5,box[1]+5),label,fill=color)
            crop=source.crop(box);background=Image.new('RGBA',crop.size,'#444');background.alpha_composite(crop)
            crop=background.convert('RGB');crop.thumbnail((460,365));x=i%3*480;y=i//3*410;sd.text((x+10,y+10),label,fill=color);sheet.paste(crop,(x+10,y+35));records.append(dict(label=label,bbox=box,source_crop_only=True))
        marked.save(output/'labeled-source.png');sheet.save(output/'source-contact-sheet.png')
        diagnostics=[]
        crowns=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('projection_component')=='crown' and o.get('asset_group','').startswith('croisement02-tree-')]
        for i,(label,box,z) in enumerate(REGIONS[:2]):
            cx=(box[0]+box[2])/2;cy=(box[1]+box[3])/2;target=Vector((cx,-(cy+z*COS)/SIN,z))
            for mode in ['full-context','tree-crowns-hidden']:
                hidden=[(o,o.hide_render) for o in crowns]
                if mode=='tree-crowns-hidden':
                    for o in crowns:o.hide_render=True
                camera(scene,target,Vector((.55,-.79,.27)).normalized(),1000,700,680)
                path=output/f'contact-{i}-{mode}.png';scene.render.filepath=str(path)
                if not args.diagnostics_only or mode!='full-context':bpy.ops.render.render(write_still=True,scene=scene.name)
                for o,value in hidden:o.hide_render=value
                diagnostics.append(dict(label=label,mode=mode,image=path.name,sha256=sha(path),note='Only tree crowns hidden for contact visibility; source-camera and full-context views retain all visible scene geometry.' if mode=='tree-crowns-hidden' else 'All visible scene geometry retained.'))
        if sha(stage/'scene.blend')!=evidence['model_sha256']:raise ValueError('Rendering changed frozen scene')
        write_json(output/'evidence.json',dict(model_sha256=evidence['model_sha256'],source_camera=dict(size=[1792,1152],projection='Native35degree orthographic pixel registration'),regions=records,diagnostics=diagnostics,images={p.name:sha(p) for p in output.glob('*.png')},visual_review='pending'))
    finally:release()


if __name__=='__main__':main()

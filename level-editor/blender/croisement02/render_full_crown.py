"""Supplement frozen review views with explicitly wider full-crown framing."""
import argparse
import json
import sys
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,tree_workspace
from render_slots import acquire,release
from evidence_io import sha,write_json
from render_multiview_asset import render


def main():
    parser=argparse.ArgumentParser();parser.add_argument('masks',nargs='+',type=int)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    try:
        for mask in args.masks:
            w=tree_workspace(mask);dest=w/'inspection/full-crown';dest.mkdir(exist_ok=True)
            acquire();before=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
            scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
            packet=json.loads((w/'modified/views.json').read_text())
            for view in packet['views']:
                view['ortho_scale']*=1.5;view['crop']=dict(width=packet['tile_size'][0],height=packet['tile_size'][1])
            write_json(dest/'cameras.json',packet)
            render(dest/'cameras.json',dest,modes=('solid','textured'),width=384)
            for mode in ['solid','textured']:
                images=[Image.open(dest/f'view-{i}-{mode}.png').convert('RGB') for i in range(8)];width,height=images[0].size
                sheet=Image.new('RGB',(width*4,height*2))
                for i,image in enumerate(images):sheet.paste(image,((i%4)*width,(i//4)*height))
                sheet.save(dest/f'{mode}.png')
            assert before==sha(w/'model.blend')
            write_json(dest/'evidence.json',dict(model_sha256=before,original_cameras_sha256=sha(w/'modified/views.json'),supplemental_cameras_sha256=sha(dest/'cameras.json'),scale_factor=1.5,solid_sha256=sha(dest/'solid.png'),textured_sha256=sha(dest/'textured.png')))
            release()
    finally:release()

if __name__=='__main__':main()

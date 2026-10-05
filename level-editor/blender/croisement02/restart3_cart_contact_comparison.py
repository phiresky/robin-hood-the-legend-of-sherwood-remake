"""Render fixed close contact views before and after the cart timber correction."""
import argparse
import json
import sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate',type=Path)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    before=OUT/'restart3-south-cart/broken-panel-v2'
    after=args.candidate.resolve();dest=after/'contact-comparison-v1'
    assert not dest.exists()
    directions=[RAY,Vector((1,-1,.25)).normalized(),Vector((0,1,.3)).normalized(),Vector((1,0,.8)).normalized()]
    acquire()
    try:
        dest.mkdir()
        rows=[];center=None
        for label,worker in [('before',before),('after',after)]:
            model=worker/'worker.blend';digest=sha(model)
            bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
            if center is None:
                wheel=scene.objects['Wheel 0 1']
                center=sum((wheel.matrix_world@v.co for v in wheel.data.vertices),Vector())/len(wheel.data.vertices)
            camera=scene.camera;camera.data.ortho_scale=70
            scene.render.resolution_x=scene.render.resolution_y=512
            scene.view_layers[0].material_override=scene.objects['Tipped barrel canopy shell'].data.materials[1]
            for index,direction in enumerate(directions):
                camera.location=center+direction*3000
                camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
                scene.render.filepath=str(dest/f'{label}-{index}.png');bpy.ops.render.render(write_still=True)
            rows.append(dict(worker=str(worker),model_sha256=digest))
        sheet=Image.new('RGBA',(2048,1024))
        for row,label in enumerate(['before','after']):
            for col in range(4):sheet.paste(Image.open(dest/f'{label}-{col}.png'),(col*512,row*512))
        sheet.save(dest/'solid-before-after.png')
        write_json(dest/'manifest.json',dict(rows=rows,center=list(center),directions=[list(d) for d in directions],
            top_left_original_game_direction=True,scope='Supplemental wheel/board contact closeups; deliberate crop of whole cart',
            layout='Top row before; bottom row after, identical cameras'))
    finally:release()


if __name__=='__main__':main()

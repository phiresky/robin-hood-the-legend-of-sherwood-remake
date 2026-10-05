"""Render unclipped, pixel-aligned native source comparisons for cargo candidates."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    acquire()
    try:
        names=sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else ['barrel-v1','loose-wood-v1']
        for name in names:
            box=[1148,765,1224,841] if name.startswith('barrel') else [1000,950,1080,1030]
            worker=OUT/'restart3-south-cart'/name;metadata=json.loads((worker/'manifest.json').read_text())
            dest=worker/'native-comparison-v1';dest.mkdir(exist_ok=False)
            assert sha(worker/'worker.blend')==metadata['model_sha256']
            source_box=metadata['source_box'];size=box[2]-box[0];scale=6
            for label,path in [('context',Path(metadata['source_frame']['image'])),('scoped',worker/'source.png')]:
                canvas=Image.new('RGBA',(size,size));im=Image.open(path).convert('RGBA')
                canvas.alpha_composite(im,(source_box[0]-box[0],source_box[1]-box[1]))
                canvas.resize((size*scale,size*scale),Image.Resampling.NEAREST).save(dest/f'{label}.png')
            bpy.ops.wm.open_mainfile(filepath=str(worker/'worker.blend'));scene=bpy.context.scene
            camera=scene.camera;camera.data.sensor_fit='HORIZONTAL';camera.data.ortho_scale=size
            center=point((box[0]+box[2])/2,(box[1]+box[3])/2,0)
            camera.location=center+RAY*3000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
            scene.render.resolution_x=scene.render.resolution_y=size*scale
            gray=next(o for o in scene.objects if o.type=='MESH').data.materials[1]
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=gray if mode=='solid' else None
                scene.render.filepath=str(dest/f'{mode}.png');bpy.ops.render.render(write_still=True)
            sheet=Image.new('RGBA',(size*scale*4,size*scale))
            for index,label in enumerate(['context','scoped','actual','solid']):sheet.paste(Image.open(dest/f'{label}.png'),(size*scale*index,0))
            sheet.save(dest/'comparison.png')
            expected=np.asarray(Image.open(dest/'scoped.png'))[:,:,3]>127
            actual=np.asarray(Image.open(dest/'actual.png'))[:,:,3]>127
            write_json(dest/'report.json',dict(model_sha256=metadata['model_sha256'],native_box=box,scale=scale,
                layout='original native context, scoped native pixels, actual saved model, solid saved model',
                source_iou=float((expected&actual).sum()/(expected|actual).sum()),missing_render_pixels=int((expected&~actual).sum()),
                extra_render_pixels=int((actual&~expected).sum()),native_direction=list(RAY),
                replaces_clipped_portrait_native_diagnostic=name=='barrel-v1',
                limitations=['Scoped silhouette measurement only; inferred thickness creates small margins beyond native pixels.',
                            'Gray surfaces and nearby unassigned native marks remain explicit.']))
    finally:release()


if __name__=='__main__':main()

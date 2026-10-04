"""Render receiver IDs for a private upper-cliff texture continuation diagnostic."""
from pathlib import Path
import sys
import json

import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'),
               str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from render_multiview_asset import render
from bake_texture_candidate import preflight


def main():
    experiment = OUT/'texture-fill-round-2/croisement02-northwest-rock-outcrop/complete-preparation/experiment-cliff-retry-v2'
    output = experiment.parent/'upper-cliff-scope-v1'
    if output.exists():
        raise FileExistsError(output)
    acquire()
    try:
        manifest, scene, names, _, report = preflight(experiment)
        upper = [name for name in names if name.endswith('part 035')]
        if len(upper) != 1:
            raise ValueError('Expected exact main-cliff receiver035')
        for name in names:
            obj = scene.objects[name]
            mat = bpy.data.materials.new('Receiver diagnostic ' + name)
            mat.use_nodes = True
            mat.node_tree.nodes.clear()
            color = mat.node_tree.nodes.new('ShaderNodeEmission')
            color.inputs['Color'].default_value = (1,0,0,1) if name in upper else (0,1,0,1)
            out = mat.node_tree.nodes.new('ShaderNodeOutputMaterial')
            mat.node_tree.links.new(color.outputs[0],out.inputs[0])
            obj.data.materials.clear()
            obj.data.materials.append(mat)
            for face in obj.data.polygons:
                face.material_index = 0
        scene.cycles.samples = 8
        scene.cycles.use_denoising = False
        output.mkdir()
        render(experiment/'views.json', output/'ids', width=manifest['tile_size'][0])
        w,h = manifest['tile_size']
        full = np.zeros((h*2,w*4),bool)
        rows=[]
        for i in range(8):
            rgba=np.asarray(Image.open(output/'ids'/f'view-{i}-textured.png').convert('RGBA'))
            mask=(rgba[:,:,0]>240)&(rgba[:,:,1]<15)
            full[(i//4)*h:(i//4+1)*h,(i%4)*w:(i%4+1)*w]=mask
            Image.fromarray(mask.astype('uint8')*255).save(output/f'upper-{i}.png')
            rows.append(dict(view=i,upper_pixels=int(mask.sum())))
        Image.fromarray(full.astype('uint8')*255).save(output/'upper-atlas.png')
        write_json(output/'scope.json',dict(status='receiver-ID diagnostic only',
            approved_model_sha256=sha(experiment/'approved-model.blend'),
            views_sha256=sha(experiment/'views.json'), upper_receiver=upper[0],
            rows=rows, geometry_saved=False, preflight=report))
    finally:
        release()


if __name__=='__main__':
    main()

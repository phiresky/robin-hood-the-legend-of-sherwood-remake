"""Render the frozen baseline beside a prototype at its exact review cameras."""
import argparse
import json
import sys
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from render_multiview_asset import render
from evidence_io import sha,write_json


def main(worker):
    output=worker/'inspection/baseline-comparison'
    if output.exists():raise ValueError('Comparison already exists')
    acquire()
    try:
        baseline=worker/'baseline.blend'
        model_hash=sha(worker/'model.blend')
        bpy.ops.wm.open_mainfile(filepath=str(baseline))
        scene=bpy.data.scenes['Croisement02 Refinement']
        scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
        scene.world=bpy.data.worlds.new('Neutral actual-material inspection');scene.world.color=(.10,.10,.10)
        manifest=worker/'inspection/actual-camera-manifest.json'
        render(manifest,output,width=384)
        for offset in (0,4):
            sheet=Image.new('RGB',(1536,768))
            for i in range(4):
                name=f'view-{i+offset}-textured.png'
                sheet.paste(Image.open(output/name).convert('RGB'),(i*384,0))
                sheet.paste(Image.open(worker/'inspection/actual-materials'/name).convert('RGB'),(i*384,384))
            sheet.save(output/f'comparison-{offset}-{offset+3}.png')
        if sha(worker/'model.blend')!=model_hash:raise ValueError('Prototype changed during comparison')
        write_json(output/'evidence.json',dict(baseline_sha256=sha(baseline),model_sha256=model_hash,
            camera_manifest_sha256=sha(manifest),layout='Prior approved geometry above; isolated prototype below. Identical cameras.',
            sheets={p.name:sha(p) for p in output.glob('comparison-*.png')}))
    finally:release()

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('worker',type=Path)
    main(parser.parse_args(sys.argv[sys.argv.index('--')+1:]).worker.resolve())

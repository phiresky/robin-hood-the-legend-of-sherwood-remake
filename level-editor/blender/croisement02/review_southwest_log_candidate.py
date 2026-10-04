"""Compare the unchanged approved log bake with its additive candidate."""
import argparse
import json
import shutil
import sys
from pathlib import Path
import bpy
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from source_coverage import audit
from complete_southwest_logs import ASSET,verify_saved


def main(worker):
    verify_saved(worker)
    baseline=OUT/'texture-fill-round-1'/ASSET/'experiment/bake-v1/worker.blend'
    comparison=worker/'inspection/approved-baseline-comparison';comparison.mkdir(exist_ok=False)
    baseline_worker=comparison/'baseline';(baseline_worker/'inspection').mkdir(parents=True)
    shutil.copyfile(baseline,baseline_worker/'model.blend')
    report=json.loads((worker/'inspection/refinement.json').read_text());report['model_sha256']=sha(baseline)
    write_json(baseline_worker/'inspection/refinement.json',report)
    bpy.ops.wm.open_mainfile(filepath=str(baseline))
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==ASSET]
    before=audit(baseline_worker,objects);after=json.loads((worker/'inspection/source-coverage/report.json').read_text())
    if before['source_crop']!=after['source_crop']:raise ValueError('Source camera framing differs')
    source=Image.open(worker/'inspection/source-coverage/source.png').convert('RGBA')
    original=Image.open(baseline_worker/'inspection/source-coverage/render.png').convert('RGBA')
    changed=Image.open(worker/'inspection/source-coverage/render.png').convert('RGBA')
    scale=3;w,h=source.size;sheet=Image.new('RGB',(w*scale,(h*scale+24)*5),'#aaa');draw=ImageDraw.Draw(sheet)
    rows=[('Original native artwork',source),('Approved model and fill; unchanged baseline',original),('Additive candidate; existing geometry and fill preserved',changed),('Approved baseline over source',Image.alpha_composite(source,original)),('Candidate over source',Image.alpha_composite(source,changed))]
    for n,(label,image) in enumerate(rows):
        draw.text((5,n*(h*scale+24)+5),label,fill='black');resized=image.resize((w*scale,h*scale),Image.Resampling.NEAREST);sheet.paste(resized,(0,n*(h*scale+24)+24),resized)
    sheet.save(comparison/'comparison.png')
    write_json(comparison/'comparison.json',dict(model_sha256=sha(worker/'model.blend'),approved_bake_sha256=sha(baseline),comparison_sha256=sha(comparison/'comparison.png'),before=before,after=after,source_camera='Identical native35degree orthographic camera and crop',scope='Original on-disk geometry and fill remain unchanged; candidate contains the same original meshes plus added timber.'))
    print(comparison/'comparison.png',flush=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('workspace',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);acquire()
    try:main(args.workspace.resolve())
    finally:release()

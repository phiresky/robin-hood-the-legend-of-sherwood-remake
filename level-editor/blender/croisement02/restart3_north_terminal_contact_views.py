"""Render read-only cart contact views with transform-verified current receivers."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from evidence_io import sha,write_json
from tree_geometry import RAY
from log_trap_state_candidate import point
from sign_context_import import append_verified
from restart2_rebind_state_receivers import MODELS
from render_slots import acquire,release


def main():
    worker=Path(sys.argv[sys.argv.index('--')+1]);dest=worker/'contact-views-v1';dest.mkdir(exist_ok=False);model=worker/'worker.blend';digest=sha(model)
    audit=json.loads((worker/'terminal-audit-v1/receiver-raw/report.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;receipts=[]
    for label,path,expected_hash in MODELS:
        assert sha(path)==expected_hash;expected={r['object']:r for r in audit['receivers'] if r['receiver']==label}
        objects,rows=append_verified(scene,path,list(expected),expected);receipts.append(dict(receiver=label,model_sha256=expected_hash,objects=rows))
    scene.cycles.samples=16;scene.view_layers[0].material_override=None;scene.render.resolution_percentage=100;scene.camera.data.sensor_fit='HORIZONTAL'
    center=Vector((1328,-580,35));views=[]
    for name,target,direction,scale in [('native',point(1328,293,0),RAY,280),('near-low',center,Vector((.65,-1,.32)).normalized(),275),('reverse-low',center,Vector((-.8,.9,.25)).normalized(),275)]:
        camera=scene.camera;camera.location=target+direction*3000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=scale
        scene.render.resolution_x=1024;scene.render.resolution_y=768;scene.render.filepath=str(dest/(name+'.png'));bpy.ops.render.render(write_still=True);views.append(dict(name=name,target=list(target),direction=list(direction),ortho_scale=scale))
    sheet=Image.new('RGBA',(3072,768))
    for i,row in enumerate(views):sheet.paste(Image.open(dest/(row['name']+'.png')),(i*1024,0))
    sheet.save(dest/'sheet.png');assert sha(model)==digest;write_json(dest/'receipt.json',dict(model_sha256=digest,receivers=receipts,views=views,native_first=True,worker_unchanged=True))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()

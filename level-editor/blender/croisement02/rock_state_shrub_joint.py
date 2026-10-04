"""Inspect complete initial boulders behind the source-owned shrub candidate."""
import json
import sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point,sha
from render_slots import acquire,release


def main():
    base=OUT/'rock-trap-state-candidate-v8'
    shrub=OUT/'understory-round-11/assets/croisement02-shrub-62'
    audit=json.loads((shrub/'inspection/saved-model-audit.json').read_text())
    assert sha(shrub/'model.blend')==audit['model_sha256']=='5ce1f4150a51ef0cf294ff42b2b9daa2022ac6ffb586d18f7346e48b6ac38ecb'
    model=json.loads((base/'manifest.json').read_text())
    assert sha(base/'worker.blend')==model['model_sha256']
    contact=json.loads((base/'contact-audit.json').read_text());assert contact['status'].startswith('surface clearance pass');assert contact['model_sha256']==model['model_sha256'];assert len(contact['saved_reopened_receiver_proof'])==5
    dest=OUT/'rock-state-shrub-joint-v2';dest.mkdir(exist_ok=False)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'))
        scene=bpy.context.scene
        rocks=[o for o in scene.objects if o.get('state_endpoint')=='covered']
        for obj in scene.objects:
            if obj.get('state_endpoint'):obj.hide_render=obj not in rocks
        names=[r['object']for r in audit['objects']]
        with bpy.data.libraries.load(str(shrub/'model.blend'),link=False)as(src,dst):dst.objects=list(names)
        for obj in dst.objects:scene.collection.objects.link(obj)
        bpy.context.view_layer.update()
        scene.cycles.transparent_max_bounces=256;scene.cycles.samples=24
        source=json.loads((OUT/'state-target-evidence/rock-trap/manifest.json').read_text());left,top,right,bottom=source['bbox']
        target=point((left+right)/2,(top+bottom)/2,0)
        for with_shrub in (False,True):
            for obj in dst.objects:obj.hide_render=not with_shrub
            for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.8)).normalized())]:
                scene.camera.location=target+direction*3000;scene.camera.rotation_euler=(target-scene.camera.location).to_track_quat('-Z','Y').to_euler()
                scene.render.filepath=str(dest/f'{"joint" if with_shrub else "rocks"}-{view}-actual.png');bpy.ops.render.render(write_still=True)
        assert sha(base/'worker.blend')==model['model_sha256'];assert sha(shrub/'model.blend')==audit['model_sha256']
        (dest/'manifest.json').write_text(json.dumps(dict(status='unapproved initial-state interaction diagnostic',rock_model_sha256=model['model_sha256'],contact_audit_sha256=sha(base/'contact-audit.json'),shrub_model_sha256=audit['model_sha256'],bank_model_sha256=model['bank_model_sha256'],source_manifest_sha256=sha(OUT/'state-target-evidence/rock-trap/manifest.json'),objects=names,limitations=['Shrub geometry and initial boulder geometry remain held for review.','No runtime visibility parity claim from static joint alone.','Native source RGB, state timing and source frames preserved; no model saved.']),indent=2)+'\n')
    finally:release()


if __name__=='__main__':main()

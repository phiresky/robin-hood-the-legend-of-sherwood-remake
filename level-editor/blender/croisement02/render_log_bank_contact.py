"""Review immutable coherent log endpoints on their exact terrain receiver."""
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(Path(__file__).parent)]
from catalog import OUT
from log_trap_state_candidate import point, sha
from tree_geometry import RAY
from render_slots import acquire, release


def main():
    base = OUT/'log-trap-state-candidate-v9'
    dest = base/'bank-review'
    dest.mkdir(exist_ok=False)
    report = json.loads((base/'dense-contact-audit.json').read_text())
    assert report['status'].startswith('sampled support pass')
    assert sha(base/'worker.blend') == report['model_sha256']
    bank = OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank'
    audit = json.loads((bank/'inspection/saved-model-audit.json').read_text())
    assert sha(bank/'model.blend') == report['bank_model_sha256'] == audit['model_sha256']
    names = [r['object'] for r in audit['objects'] if r['source_node'] in [f'building-{i:03d}' for i in range(5)]]
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'))
        scene = bpy.context.scene
        logs = [o for o in scene.objects if o.get('state_endpoint') == 'applied']
        for obj in scene.objects:
            if obj.get('state_endpoint'):
                obj.hide_render = obj not in logs
        with bpy.data.libraries.load(str(bank/'model.blend'), link=False) as (src,dst):
            dst.objects = list(names)
        for obj in dst.objects:
            scene.collection.objects.link(obj)
        bpy.context.view_layer.update()
        solid = bpy.data.materials.new('Contact review solid')
        solid.diffuse_color = (.25,.25,.25,1)
        solid.use_nodes = True
        solid.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (.25,.25,.25,1)
        scene.cycles.samples = 24
        for enabled in [False,True]:
            for obj in dst.objects:
                obj.hide_render = not enabled
            for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.8)).normalized())]:
                target = point(518,559.5,0) if view == 'source' else point(518,550,25)
                scene.camera.location = target+direction*3000
                scene.camera.rotation_euler = (target-scene.camera.location).to_track_quat('-Z','Y').to_euler()
                scene.camera.data.ortho_scale = 307.2 if view == 'source' else 400
                for mode in ['actual','solid']:
                    replacements = []
                    if mode == 'solid':
                        for obj in logs+list(dst.objects):
                            for slot in obj.material_slots:
                                replacements.append((slot,slot.material))
                                slot.material = solid
                    scene.render.filepath = str(dest/f'{"bank" if enabled else "logs"}-{view}-{mode}.png')
                    bpy.ops.render.render(write_still=True)
                    for slot,material in replacements:
                        slot.material = material
        assert sha(base/'worker.blend') == report['model_sha256']
        assert sha(bank/'model.blend') == report['bank_model_sha256']
        (dest/'manifest.json').write_text(json.dumps(dict(status='unapproved physical support review; foreground crowns deliberately absent',log_model_sha256=report['model_sha256'],bank_model_sha256=report['bank_model_sha256'],receiver_objects=names),indent=2)+'\n')
    finally:
        release()


if __name__ == '__main__':
    main()

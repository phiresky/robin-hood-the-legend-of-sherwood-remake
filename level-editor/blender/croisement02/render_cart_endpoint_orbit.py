"""Review a frozen cart-only candidate from eight actual and solid viewpoints."""
import json, math, sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(Path(__file__).parent)]
from catalog import OUT
from log_trap_state_candidate import sha
from render_slots import acquire, release


def main():
    base = OUT / (sys.argv[sys.argv.index('--candidate')+1] if '--candidate' in sys.argv else 'north-cart-initial-candidate-v3')
    dest = base / 'endpoint-orbit'
    dest.mkdir(exist_ok=False)
    model = base / 'worker.blend'
    before = sha(model)
    support = json.loads((base / 'support-audit.json').read_text())
    assert support['model_sha256'] == before
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.context.scene
        scene.cycles.samples = 12
        scene.cycles.use_denoising = False
        scene.render.resolution_x = scene.render.resolution_y = 512
        objects = [o for o in scene.objects if o.type == 'MESH']
        points = [o.matrix_world @ v.co for o in objects for v in o.data.vertices]
        low = Vector(tuple(min(p[i] for p in points) for i in range(3)))
        high = Vector(tuple(max(p[i] for p in points) for i in range(3)))
        center = (low + high) / 2
        solid = bpy.data.materials.new('Private cart solid review')
        solid.use_nodes = True
        solid.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (.25, .25, .25, 1)
        records = []
        for view in range(8):
            angle = math.pi / 2 + view * math.pi / 4
            direction = Vector((math.cos(angle) * math.cos(math.radians(35)), math.sin(angle) * math.cos(math.radians(35)), math.sin(math.radians(35))))
            scene.camera.location = center + direction * 3000
            scene.camera.rotation_euler = (center - scene.camera.location).to_track_quat('-Z', 'Y').to_euler()
            scene.camera.data.ortho_scale = (high - low).length * 1.15
            for mode in ['actual', 'solid']:
                scene.view_layers[0].material_override = solid if mode == 'solid' else None
                path = dest / f'{view:02d}-{mode}.png'
                scene.render.filepath = str(path)
                bpy.ops.render.render(write_still=True)
                records.append(dict(view=view, mode=mode, image=path.name, sha256=sha(path)))
        assert sha(model) == before
        (dest / 'manifest.json').write_text(json.dumps(dict(status='private cart-only geometry review; actor and transition work incomplete', model_sha256=before, support_sha256=sha(base / 'support-audit.json'), records=records), indent=2) + '\n')
    finally:
        release()


if __name__ == '__main__':
    main()

"""Review private packed stone GLBs using the approved native-first cameras."""
import hashlib
import json
from pathlib import Path
import shutil
import sys
import bpy
from mathutils import Vector, Matrix
from mathutils.kdtree import KDTree
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release
from render_multiview_asset import render


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    assert shutil.disk_usage(ROOT).free > 25 * 1024**3
    run = ROOT / 'level-editor/work/croisement03-refinement/restart2'
    stage = run / 'approved-stone-integration-preflight/three-stones-stage-v1'
    library = stage / 'map-assets'
    out = stage / 'derivative-review-v1'
    assert not out.exists()
    index = json.loads((library / '3d-assets/index.json').read_text())
    experiments = {'croisement03-central-shrub-boulder':'texture-batch-v7',
                   'croisement03-east-tree-rocks':'texture-batch-v8',
                   'croisement03-southeast-stone-wall':'texture-wall10'}
    out.mkdir(); rows = []
    acquire()
    try:
        for entry in index['assets']:
            identity = entry['id']; experiment = run / experiments[identity] / identity / 'experiment'
            worker = experiment / 'baked-preserved-v1/worker.blend'
            model = library / '3d-assets' / entry['model']
            descriptor_path = library / '3d-assets' / entry['descriptor']
            descriptor = json.loads(descriptor_path.read_text())
            assert sha(descriptor_path) == entry['descriptor_sha256']
            bpy.ops.wm.open_mainfile(filepath=str(worker))
            manifest = json.loads((experiment / 'views.json').read_text())
            scene = bpy.data.scenes[manifest['scene_name']]; bpy.context.window.scene = scene
            expected = [o.matrix_world @ v.co for o in scene.objects
                        if o.type == 'MESH' and o.get('asset_group') == identity for v in o.data.vertices]
            tree = KDTree(len(expected))
            for i,p in enumerate(expected): tree.insert(p,i)
            tree.balance()
            previous = set(bpy.data.objects)
            for obj in scene.objects:
                if obj.type == 'MESH': obj.hide_render = True
            bpy.ops.import_scene.gltf(filepath=str(model))
            imported = set(bpy.data.objects) - previous
            meshes = [o for o in imported if o.type == 'MESH']
            assert meshes
            bpy.context.view_layer.update()
            # The GLB has its authored Z-up map wrapper. Blender's glTF import
            # restores those coordinates; only the asset origin is reapplied.
            transform = Matrix.Translation(Vector(descriptor['source_origin_scene']))
            for obj in [o for o in imported if o.parent not in imported]:
                obj.matrix_world = transform @ obj.matrix_world
            bpy.context.view_layer.update()
            actual = [o.matrix_world @ v.co for o in meshes for v in o.data.vertices]
            error = max(tree.find(p)[2] for p in actual)
            reverse = KDTree(len(actual))
            for i,p in enumerate(actual): reverse.insert(p,i)
            reverse.balance()
            error = max(error,max(reverse.find(p)[2] for p in expected))
            assert error < .001, error
            for obj in meshes: obj['asset_group'] = identity; obj.hide_render = False
            target = out / identity; target.mkdir()
            manifest.pop('render_object_names',None)
            (target / 'views.json').write_text(json.dumps(manifest,indent=2)+'\n')
            render(target / 'views.json', target / 'actual', modes=('textured','solid'), width=384)
            for mode in ('textured','solid'):
                sheet = Image.new('RGB',(1536,768))
                for i in range(8):
                    with Image.open(target / 'actual' / f'view-{i}-{mode}.png') as im:
                        sheet.paste(im.convert('RGB'),((i%4)*384,(i//4)*384))
                sheet.save(target / f'{mode}.png')
            rows.append(dict(asset=identity, model=str(model), model_sha256=sha(model),
                descriptor_sha256=sha(descriptor_path), approved_worker_sha256=sha(worker),
                imported_meshes=len(meshes), maximum_bidirectional_vertex_error=error,
                actual_sheet=str(target / 'textured.png'), actual_sheet_sha256=sha(target / 'textured.png'),
                solid_sheet=str(target / 'solid.png'), solid_sheet_sha256=sha(target / 'solid.png'),
                native_view_index=0))
        (out / 'receipt.json').write_text(json.dumps(dict(status='Saved exported-derivative views; visual review pending',
            models=rows, live_library_changed=False, limitations=['Blender reimport/render proof; browser integration still pending.',
                'These three stone appearances do not complete neighboring vegetation or terrain.']),indent=2)+'\n')
    finally:
        release()


if __name__ == '__main__':
    main()

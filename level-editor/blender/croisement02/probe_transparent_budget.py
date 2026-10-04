"""Compare declared transparent-ray budgets without changing a worker or geometry."""
import argparse
import json
import shutil
import sys
from pathlib import Path

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json
from render_slots import acquire, release
from render_multiview_asset import render
from source_coverage import audit
from render_convergence import difference


def main(worker, destination, budgets, view):
    if destination.exists():
        raise ValueError('Use a fresh diagnostic destination')
    if len(budgets) < 2 or any(not 1 <= b <= 1024 for b in budgets):
        raise ValueError('Provide at least two transparent budgets in 1..1024')
    cfg = json.loads((worker / 'workspace.json').read_text())
    model_hash = sha(worker / 'model.blend')
    cameras = worker / 'inspection/actual-camera-manifest.json'
    packet = json.loads(cameras.read_text())
    packet['views'] = [v for v in packet['views'] if v['index'] == view]
    if len(packet['views']) != 1:
        raise ValueError('Representative view must exist in the frozen camera packet')
    destination.mkdir(parents=True)
    results = []
    acquire()
    try:
        for budget in budgets:
            run = destination / f'bounces-{budget}'
            (run / 'inspection').mkdir(parents=True)
            # Read-only model link avoids another large worker copy; never save here.
            (run / 'model.blend').symlink_to(worker / 'model.blend')
            shutil.copy2(worker / 'workspace.json', run / 'workspace.json')
            shutil.copy2(worker / 'inspection/refinement.json', run / 'inspection/refinement.json')
            write_json(run / 'camera.json', packet)
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            scene = bpy.data.scenes[cfg['scene_name']]
            scene.render.engine = 'CYCLES'
            scene.cycles.samples = 4
            scene.cycles.seed = 0
            scene.cycles.transparent_max_bounces = budget
            scene.world = bpy.data.worlds.new('Neutral budget diagnostic')
            scene.world.color = (.10, .10, .10)
            render(run / 'camera.json', run / 'actual-materials', width=384)
            objects = [o for o in bpy.data.collections[cfg['collection_name']].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == cfg['asset_id']]
            audit(run, objects, transparent_bounces=budget)
            results.append(dict(budget=budget, actual=str(run / f'actual-materials/view-{view}-textured.png'),
                                native=str(run / 'inspection/source-coverage/render.png'),
                                coverage=json.loads((run / 'inspection/source-coverage/report.json').read_text())))
        if sha(worker / 'model.blend') != model_hash:
            raise RuntimeError('Diagnostic changed its source worker')
        comparisons = [dict(first=a['budget'], second=b['budget'],
                            native=difference(a['native'], b['native']), actual=difference(a['actual'], b['actual']))
                       for a, b in zip(results, results[1:])]
        files = [p for p in destination.rglob('*') if p.is_file() and not p.is_symlink()]
        write_json(destination / 'evidence.json', dict(worker=str(worker), model_sha256=model_hash,
            camera_sha256=sha(cameras), representative_view=view, samples=dict(actual=4, native=8),
            results=results, comparisons=comparisons,
            file_sha256={str(p.relative_to(destination)):sha(p) for p in files},
            interpretation='Unexpected dark opaque rays can reflect exhausted transparency budget; compare convergence separately from geometric source coverage.',
            worker_unchanged=True, approval='diagnostic only'))
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--budgets', type=int, nargs='+', default=[256, 512])
    parser.add_argument('--view', type=int, default=1)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.worker.resolve(), args.destination.resolve(), args.budgets, args.view)

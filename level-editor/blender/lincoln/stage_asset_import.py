"""Stage one newly approved asset model on top of an existing (textured) Lincoln stage.

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/stage_asset_import.py -- \
      --stage-in <publication-N/stage-vM> --asset <asset id> --model <approved model.blend> \
      --output <publication-N/stage-vK-geometry>

The approval must exist in approvals.json for the exact model hash. The stage-in approvals
snapshot (integration `approvals_snapshot`, or approvals.json) is copied with that asset's record
replaced, so later terrain/tree re-approvals stay out of this publication. Only the asset's visible
base meshes are replaced by the shared `import_asset_geometry` guard (transforms, coverage and every
outside object checked); revealed-state copies (`state_variant_of`) of the asset are kept aside
during the import and left for the states rebake (lincoln-states `--replace-assets`). Run
`global_reproject.py apply --assets <asset id>` on the output next.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage-in', type=Path, required=True)
    parser.add_argument('--asset', required=True)
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(argv)
    stage_in = args.stage_in.resolve(strict=True)
    integration = json.loads((stage_in / 'integration.json').read_text())
    if sha(stage_in / 'worker.blend') != integration['worker_sha256']:
        raise ValueError('Stage-in worker changed after integration')
    model = args.model.resolve(strict=True)
    model_sha = sha(model)
    root = Path(__file__).resolve().parents[2] / 'work/needs-review/lincoln-refinement'
    approvals = json.loads((root / 'approvals.json').read_text())['approvals']
    approval = [r for r in approvals if r['asset_id'] == args.asset and r['decision'] == 'approved'
                and r['model_sha256'] == model_sha and r['scope'] in ('geometry', 'geometry-and-states')]
    if len(approval) != 1:
        raise ValueError('Model lacks exactly one approval bound to its hash: ' + args.asset)
    snapshot_in = Path(integration['approvals_snapshot']['path']) if integration.get('approvals_snapshot') else root / 'approvals.json'
    snapshot = json.loads(snapshot_in.read_text())
    snapshot['approvals'] = [r for r in snapshot['approvals'] if r['asset_id'] != args.asset] + approval
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / 'approvals-snapshot.json').write_text(json.dumps(snapshot, indent=2) + '\n')

    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement'))
    from render_slots import acquire
    acquire()
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from freeze_tooling import select_tooling
    tooling = select_tooling()
    import bpy
    from import_reviewed_geometry import import_asset_geometry
    collection = 'lincoln Working'
    bpy.ops.wm.open_mainfile(filepath=str(model))
    owned = sorted(o.name for o in bpy.data.collections[collection].all_objects
                   if o.type == 'MESH' and not o.hide_render and o.get('asset_group') == args.asset
                   and not o.get('state_variant_of'))
    bpy.ops.wm.open_mainfile(filepath=str(stage_in / 'worker.blend'))
    bpy.context.window.scene = bpy.data.scenes['lincoln Refinement']
    held = [o for o in bpy.data.collections[collection].all_objects
            if o.get('asset_group') == args.asset and o.get('state_variant_of')]
    for obj in held:
        obj['asset_group'] = '__state_hold__'
    try:
        result = import_asset_geometry(str(model), asset_id=args.asset, object_names=owned, collection_name=collection)
    finally:
        for obj in held:
            obj['asset_group'] = args.asset
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'worker.blend'))
    shutil.copyfile(stage_in / 'catalog.json', output / 'catalog.json')
    record = {**result, 'model_sha256': model_sha,
              'approval': {k: approval[0][k] for k in ('asset_id', 'decision', 'scope', 'exact_text', 'model_sha256',
                                                       'modified_views_sha256', 'recorded_utc', 'evidence_directory')},
              'state_copies_kept_for_rebake': len(held)}
    staged = dict(integration)
    staged['imports'] = [i for i in integration['imports'] if i['asset_id'] != args.asset] + [record]
    staged['asset_import_chain'] = integration.get('asset_import_chain', []) + [
        {'stage_in': str(stage_in), 'stage_in_worker_sha256': integration['worker_sha256'], 'asset_id': args.asset,
         'model': str(model), 'model_sha256': model_sha, 'tooling': tooling['snapshot_id'],
         'script': str(Path(__file__).resolve()), 'script_sha256': sha(__file__)}]
    staged['approvals_snapshot'] = {'path': str(output / 'approvals-snapshot.json'),
                                    'sha256': sha(output / 'approvals-snapshot.json'),
                                    'base': str(snapshot_in),
                                    'reason': f'{args.asset} replaced by its approved revision; other records as in the stage-in snapshot.'}
    staged['worker'] = str(output / 'worker.blend')
    staged['worker_sha256'] = sha(output / 'worker.blend')
    staged['staged_catalog'] = str(output / 'catalog.json')
    (output / 'integration.json').write_text(json.dumps(staged, indent=2) + '\n')
    print(json.dumps({'output': str(output), 'worker_sha256': staged['worker_sha256'],
                      'replaced': len(owned), 'state_copies_kept': len(held)}), flush=True)


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1:])

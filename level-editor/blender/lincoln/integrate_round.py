"""Integrate every ready worker model into one Lincoln scene for the next workspace round.

Run from the repository root:

    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/integrate_round.py -- \
      --source level-editor/work/needs-review/lincoln-refinement/grouped/lincoln-grouped-v1.blend \
      --progress level-editor/work/needs-review/lincoln-refinement/scratch/coordinator/gallery-progress.json \
      --output level-editor/work/needs-review/lincoln-refinement/integrated/lincoln-integrated-r1.blend

Only packets that the collector validated as ready-for-user are imported, and only
when their current model hash still matches the collector evidence. Every other asset
keeps its grouped-baseline geometry and is listed in the report. The shared
`import_asset_geometry` guard rejects ownership, coverage or outside-scene changes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--progress', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--collection', default='lincoln Working')
    parser.add_argument('--statuses', default='ready-for-user',
                        help='Comma-separated collector statuses whose current models are imported')
    args = parser.parse_args(argv)
    if args.output.exists():
        raise FileExistsError(args.output)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from render_slots import acquire
    acquire()
    from freeze_tooling import select_tooling
    tooling = select_tooling()
    import bpy
    from import_reviewed_geometry import import_asset_geometry

    progress = json.loads(args.progress.read_text())
    evidence_dir = args.progress.parent / (args.progress.name.removesuffix('-progress.json') + '-packet-evidence')
    ready, skipped = [], []
    for row in progress['assets']:
        if row['status'] not in args.statuses.split(','):
            skipped.append({'asset_id': row['id'], 'status': row['status']})
            continue
        evidence = json.loads((evidence_dir / (row['id'] + '.json')).read_text())
        model = Path(row['workspace']) / 'model.blend'
        if sha(model) != evidence['model_sha256']:
            raise RuntimeError('Model changed after collection: ' + row['id'])
        ready.append((row['id'], model, evidence['model_sha256']))

    # First pass: read the owned visible mesh names from each saved worker model.
    owned = {}
    for asset_id, model, _ in ready:
        bpy.ops.wm.open_mainfile(filepath=str(model))
        owned[asset_id] = sorted(o.name for o in bpy.data.collections[args.collection].all_objects
                                 if o.type == 'MESH' and not o.hide_render and o.get('asset_group') == asset_id)
        if not owned[asset_id]:
            raise ValueError('Worker model has no visible owned meshes: ' + asset_id)

    source_hash = sha(args.source)
    bpy.ops.wm.open_mainfile(filepath=str(args.source.resolve()))
    bpy.context.window.scene = bpy.data.scenes['lincoln Refinement']
    imports = []
    for asset_id, model, model_hash in ready:
        result = import_asset_geometry(model, asset_id=asset_id, object_names=owned[asset_id],
                                       collection_name=args.collection)
        imports.append({**result, 'model_sha256': model_hash})
        print('IMPORTED', asset_id, flush=True)
    # Each worker model packs its own copy of shared source images (the 8192² synthesized atlas,
    # the refreshed source); remap byte-identical packed images so the scene stays tractable.
    by_content = {}
    remapped = 0
    for image in list(bpy.data.images):
        if not image.packed_file:
            continue
        key = hashlib.sha256(image.packed_file.data).hexdigest()
        keeper = by_content.setdefault(key, image)
        if keeper is not image:
            image.user_remap(keeper)
            bpy.data.images.remove(image)
            remapped += 1
    bpy.data.orphans_purge(do_local_ids=True, do_linked_ids=True, do_recursive=True)
    if sha(args.source) != source_hash:
        raise RuntimeError('Grouped source changed during integration')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(args.output.resolve()))
    report = {'version': 1, 'source': str(args.source.resolve()), 'source_sha256': source_hash,
              'progress': str(args.progress.resolve()), 'progress_sha256': sha(args.progress),
              'output_sha256': sha(args.output), 'tooling': tooling['snapshot_id'],
              'imported': imports, 'baseline_retained': skipped,
              'deduplicated_packed_images': remapped, 'unique_packed_images': len(by_content),
              'scope': 'Geometry integration for the next review round; not user approval.'}
    args.output.with_suffix('.integration.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'imported': len(imports), 'baseline_retained': skipped}))


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1:])

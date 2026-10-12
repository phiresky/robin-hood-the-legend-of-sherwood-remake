"""Publish the exact derived models checked by the browser visual audit.

Existing source assets and scenes are never rewritten. Run build_segments.py,
then render-audit.mjs --segments --force, and inspect the comparison images first.
"""
import json
import argparse
import shutil
import subprocess

from build_segments import ROOT, LIB, STAGE, sha, write_asset_index
from asset_index import generate_asset_index
from lossy_assets import LibraryLock, verify_derivatives


def prepare_runtime(ids, *, blender='blender', stage=STAGE):
    """Derive optimized browser payloads before installing any reviewed copy."""
    subprocess.run([
        blender, '--background', '--threads', '2', '--python-exit-code', '1',
        '--python', str(ROOT / 'refinement/blender/lossy_assets.py'), '--', 'refresh',
        '--root', str(stage), '--work', str(ROOT / 'work/needs-review/wall-presets/runtime'),
        '--assets', *ids,
    ], check=True)
    index = generate_asset_index(stage)
    problems = verify_derivatives(stage, index=index)
    if problems:
        raise ValueError(f'Invalid runtime derivatives after refresh: {problems}')
    entries = {entry['id']: entry for entry in index['assets']}
    files = {}
    for identity in ids:
        entry = entries[identity]
        names = [entry['model'], entry['descriptor']]
        for kind in ('lossy_model', 'preview_model'):
            if not entry.get(kind):
                raise ValueError(f'Missing runtime derivative after refresh: {identity}: {kind}')
            names.extend([entry[kind], entry[kind] + '.receipt.json'])
        files[identity] = names
    return files


def publish(ids=None, *, blender='blender'):
    rows = json.loads((ROOT / 'work/needs-review/wall-presets/segments.json').read_text())
    all_rows = rows
    if ids:
        if set(ids)-{row['id'] for row in rows}:raise ValueError('Unknown preset id')
        rows = [row for row in rows if row['id'] in ids]
    entries = {entry['id']: entry for entry in json.loads((LIB / 'index.json').read_text())['assets']}
    reviewed = {}
    for row in rows:
        folder = STAGE / row['id']
        audit = json.loads((ROOT / f"work/needs-review/wall-presets/segments/{row['id']}.json").read_text())
        if audit.get('model_sha256') != sha(folder / 'model.glb') or not audit.get('vertices'):
            raise ValueError(f"Missing or stale visual audit: {row['id']}")
        if any(audit.get(key) != value for key, value in row.items()):
            raise ValueError(f"Preset parameters changed since visual audit: {row['id']}")
        descriptor = json.loads((folder / 'asset.json').read_text())
        provenance = descriptor['provenance']
        source = entries[row['source']]
        if provenance['model_sha256'] != sha(LIB / source['model']):
            raise ValueError(f"Source model changed during review: {row['source']}")
        if provenance['descriptor_sha256'] != sha(LIB / source['descriptor']):
            raise ValueError(f"Source descriptor changed during review: {row['source']}")
        reviewed.update({
            folder / 'model.glb': audit['model_sha256'],
            folder / 'asset.json': sha(folder / 'asset.json'),
            LIB / source['model']: provenance['model_sha256'],
            LIB / source['descriptor']: provenance['descriptor_sha256'],
        })
    # A local installation must also be ready for library:publish. Do not
    # replace live models until derivation and receipt validation succeed.
    runtime_files = prepare_runtime([row['id'] for row in rows], blender=blender)
    with LibraryLock(LIB):
        for path, expected in reviewed.items():
            if sha(path) != expected:
                raise ValueError(f'Reviewed asset changed during runtime derivation: {path}')
        for row in rows:
            target = LIB / row['id']
            target.mkdir(exist_ok=True)
            for relative in runtime_files[row['id']]:
                shutil.copy2(STAGE / relative, LIB / relative)
        write_asset_index(LIB)
    fields = {'name', 'asset', 'width', 'repeatLength', 'axis', 'sourceAngle',
              'sourceStraight', 'sourceStart', 'sourceEnd', 'source_map',
              'cornerAsset', 'cornerScale', 'cornerMinAngle'}
    catalog_path = ROOT / 'app/src/assets/wall-presets.json'
    previous = {row['asset']: row for row in json.loads(catalog_path.read_text())} if ids else {}
    catalog = [previous[row['id']] if ids and row['id'] not in ids else
               {key: value for key, value in row.items() if key in fields} for row in all_rows]
    (ROOT / 'app/src/assets/wall-presets.json').write_text(json.dumps(catalog, indent=2) + '\n')
    print(f"Published {len(rows)} reviewed copies; original assets and scenes unchanged.")


if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--ids',nargs='+',help='Publish only these reviewed presets')
    parser.add_argument('--blender',default='blender',help='Blender executable for runtime derivatives')
    args=parser.parse_args()
    publish(args.ids, blender=args.blender)

"""Strict selection of a newly reviewed stem without inheriting old approval."""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json
from catalog_schema import source_for_part

ASSET = 'croisement02-supplemental-wood-44'


def validate_worker(worker):
    model = sha(worker / 'model.blend')
    records = {name: json.loads((worker / name).read_text()) for name in (
        'validation.json', 'inspection/saved-model-audit.json', 'inspection/source-coverage/report.json',
        'inspection/local-depth.json', 'inspection/actual-materials/evidence.json', 'inspection/visual-review.json')}
    if records['validation.json']['status'] != 'PASS' or records['inspection/saved-model-audit.json']['status'] != 'PASS':
        raise ValueError('Stem candidate validation failed')
    if any(r['model_sha256'] != model for name, r in records.items() if name != 'validation.json'):
        raise ValueError('Stale stem candidate model evidence')
    if records['inspection/source-coverage/report.json']['intersection_over_union'] < .95:
        raise ValueError('Stem candidate lost native silhouette')
    trunk = [r for r in records['inspection/local-depth.json']['rows'] if 1120 <= r['source_y'] <= 1150]
    if len(trunk) != 4 or min(r['depth_width_ratio'] for r in trunk) < .9:
        raise ValueError('Stem candidate main trunk is implausibly thin')
    if not records['inspection/visual-review.json'].get('ready_for_geometry_review'):
        raise ValueError('Stem candidate requires current manual review')
    sheet = sha(worker / 'inspection/actual-materials/sheet.png')
    if any(records[n]['sheet_sha256'] != sheet for n in ('inspection/actual-materials/evidence.json', 'inspection/visual-review.json')):
        raise ValueError('Stem candidate actual view changed')


def selected_workspace(out, asset, catalog):
    if asset != ASSET:
        return None
    path = out / 'stem-cleanup-selections/stem-44.json'
    if not path.exists():
        return None
    receipt = json.loads(path.read_text())
    if receipt['asset_id'] != asset or receipt['approval'] != 'pending':
        raise ValueError('Invalid stem selection')
    group = next(g for g in json.loads(catalog.read_text())['groups'] if g['id'] == asset)
    if {source_for_part(p) for p in group['parts']} != set(receipt['part_ids']) or receipt['part_ids'] != ['foliage-wood-044']:
        raise ValueError('Stem source ownership changed')
    for file, expected in receipt['evidence_sha256'].items():
        if sha(Path(file)) != expected:
            raise ValueError('Stem selection evidence changed: ' + file)
    worker = Path(receipt['worker'])
    if sha(worker / 'model.blend') != receipt['model_sha256']:
        raise ValueError('Stem selection model changed')
    validate_worker(worker)
    return worker


def expose(worker):
    from catalog import OUT, reviewed_catalog
    validate_worker(worker)
    cfg = json.loads((worker / 'workspace.json').read_text())
    if cfg['asset_id'] != ASSET or cfg['part_ids'] != ['foliage-wood-044']:
        raise ValueError('Only joined stem44 is supported')
    previous = json.loads((worker / 'inspection/prototype-preservation.json').read_text())
    old = Path(previous['previous_worker']) / 'model.blend'
    if sha(old) != previous['previous_model_sha256'] or not previous['previous_model_unchanged']:
        raise ValueError('Original approved stem changed')
    paths = [old, worker / 'model.blend', worker / 'baseline.blend', worker / 'workspace.json',
             worker / 'source-masks.json', worker / 'validation.json']
    for directory in ('inspection', 'recipe', 'reference', 'mask-reference', 'input', 'modified'):
        paths.extend(p for p in sorted((worker / directory).rglob('*')) if p.is_file())
    native = json.loads((worker / 'mask-reference/native-hashes.json').read_text())
    for file, expected in native.items():
        if sha(Path(file)) != expected:
            raise ValueError('Native stem mask changed')
        paths.append(Path(file))
    receipt = dict(asset_id=ASSET, worker=str(worker), model_sha256=sha(worker / 'model.blend'),
        approval='pending', part_ids=cfg['part_ids'], evidence_sha256={str(p): sha(p) for p in paths},
        rationale='Coordinator reviewed the joined geometry; old geometry and texture approvals do not apply.')
    target = OUT / 'stem-cleanup-selections/stem-44.json'
    if target.exists():
        raise ValueError('Archive the previous selection explicitly before replacing it')
    target.parent.mkdir(exist_ok=True)
    write_json(target, receipt)
    if selected_workspace(OUT, ASSET, reviewed_catalog()) != worker:
        raise ValueError('Stem selection failed')
    print(target)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    expose(parser.parse_args().worker.resolve())

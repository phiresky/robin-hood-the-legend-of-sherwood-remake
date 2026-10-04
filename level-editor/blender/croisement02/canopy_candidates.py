"""Select explicitly reviewed cleanup candidates without reusing prior approvals."""
import argparse
import json
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json
from catalog_schema import source_for_part


def selected_workspace(out, mask, catalog_path):
    from wood_revision_candidates import selected_workspace as wood_revision
    wood = wood_revision(out, mask, catalog_path)
    if wood is not None:
        return wood
    receipt = out / 'canopy-cleanup-selections' / f'tree-{mask:02}.json'
    if not receipt.exists():
        return None
    record = json.loads(receipt.read_text())
    asset = f'croisement02-tree-{mask:02}'
    worker = Path(record['worker'])
    if record['asset_id'] != asset or record['approval'] != 'pending':
        raise ValueError('Invalid canopy cleanup selection')
    group = next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id'] == asset)
    scope = {source_for_part(p) for p in group['parts']}
    if scope != set(record['part_ids']):
        raise ValueError('Canopy cleanup source ownership changed')
    for path, expected in record['evidence_sha256'].items():
        if sha(Path(path)) != expected:
            raise ValueError('Canopy cleanup evidence changed: ' + path)
    previous=Path(record['previous_worker'])
    if previous==worker or sha(previous/'model.blend')!=record['previous_model_sha256']:
        raise ValueError('Independent prior canopy worker changed')
    if sha(worker / 'model.blend') != record['model_sha256']:
        raise ValueError('Canopy cleanup candidate model changed')
    review = json.loads((worker / 'inspection/visual-review.json').read_text())
    if not review.get('ready_for_geometry_review') or review['model_sha256'] != record['model_sha256']:
        raise ValueError('Canopy cleanup has no current manual review')
    validate_worker(worker)
    return worker


def validate_worker(worker):
    model_hash=sha(worker/'model.blend')
    validation=json.loads((worker/'validation.json').read_text())
    audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
    coverage=json.loads((worker/'inspection/source-coverage/report.json').read_text())
    bounds=json.loads((worker/'inspection/actual-materials/opacity-bounds.json').read_text())
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    if validation['status']!='PASS' or audit['status']!='PASS':
        raise ValueError('Failed cleanup validation')
    if any(r['model_sha256']!=model_hash for r in (audit,coverage,bounds,review)):
        raise ValueError('Stale cleanup model evidence')
    if coverage['intersection_over_union']<.95 or min(c['depth_width_ratio'] for c in bounds['crowns'])<1:
        raise ValueError('Failed cleanup source or volume limits')
    if not review.get('ready_for_geometry_review'):
        raise ValueError('Cleanup requires manual review')
    preservation=json.loads((worker/'inspection/prototype-preservation.json').read_text())
    root_base=preservation.get('root_completion_base')
    if root_base:
        if not preservation.get('non_crown_geometry_and_materials_preserved'):
            raise ValueError('Combined crown candidate lost its root preservation proof')
        for path,expected in root_base['evidence_sha256'].items():
            if sha(Path(path))!=expected:
                raise ValueError('Combined root evidence changed: '+path)
        root_proof=json.loads((worker/'inspection/root-preservation.json').read_text())
        root_coverage=json.loads((worker/'inspection/root-source-coverage/report.json').read_text())
        if (root_proof['model_sha256']!=model_hash or not root_proof['preserved'] or
                root_proof['previous_meshes']!=root_proof['current_meshes'] or
                root_coverage['model_sha256']!=model_hash or root_coverage['source_coverage']<.95):
            raise ValueError('Combined root preservation or local coverage failed')


def expose(worker):
    from catalog import OUT, reviewed_catalog
    validate_worker(worker)
    cfg = json.loads((worker / 'workspace.json').read_text())
    asset = cfg['asset_id']
    mask = int(asset.rsplit('-', 1)[1])
    paths = [worker / name for name in ['model.blend', 'workspace.json', 'validation.json',
        'inspection/refinement.json', 'inspection/visual-review.json', 'inspection/prototype-preservation.json',
        'inspection/source-coverage/report.json', 'inspection/saved-model-audit.json',
        'inspection/actual-materials/evidence.json', 'inspection/actual-materials/opacity-bounds.json',
        'inspection/actual-materials/sheet.png', 'inspection/baseline-comparison/evidence.json',
        'inspection/baseline-comparison/comparison-0-3.png', 'inspection/baseline-comparison/comparison-4-7.png',
        'modified/views.json', 'modified/solid.png', 'modified/textured.png']]
    paths.extend(sorted((worker / 'recipe').glob('*.py')))
    paths.extend(sorted((worker / 'inspection/source-packet').glob('*')))
    for name in ['inspection/root-preservation.json', 'inspection/root-source-coverage/report.json']:
        if (worker/name).exists():paths.append(worker/name)
    paths.extend([worker/'baseline.blend',worker/'source-masks.json'])
    paths.extend(worker/'reference'/name for name in cfg['reference_files'])
    paths.extend(worker/'mask-reference'/name for name in cfg['mask_reference_files'])
    native=json.loads((worker/'mask-reference/native-hashes.json').read_text())
    for path,expected in native.items():
        if sha(Path(path))!=expected:raise ValueError('Source mask changed: '+path)
        paths.append(Path(path))
    previous = json.loads((worker / 'inspection/prototype-preservation.json').read_text())
    old = Path(previous['previous_worker']) / 'model.blend'
    if sha(old) != previous['previous_model_sha256']:
        raise ValueError('Earlier approved worker changed')
    paths.append(old)
    record = dict(asset_id=asset, worker=str(worker), model_sha256=sha(worker / 'model.blend'),
        part_ids=cfg['part_ids'], approval='pending',
        previous_worker=previous['previous_worker'], previous_model_sha256=previous['previous_model_sha256'],
        evidence_sha256={str(path): sha(path) for path in paths},
        rationale='Coordinator reviewed the new geometry prototype; previous geometry approval does not apply.')
    receipt = OUT / 'canopy-cleanup-selections' / f'tree-{mask:02}.json'
    receipt.parent.mkdir(exist_ok=True)
    if receipt.exists():
        raise ValueError('Existing cleanup selection must be explicitly archived before replacement')
    write_json(receipt, record)
    if selected_workspace(OUT, mask, reviewed_catalog()) != worker:
        raise ValueError('Failed cleanup selection')
    print(receipt)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    expose(parser.parse_args().worker.resolve())

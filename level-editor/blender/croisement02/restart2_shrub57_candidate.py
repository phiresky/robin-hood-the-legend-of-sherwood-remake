"""Pin the explicitly approved sign-contact shrub without inheriting old reviews."""
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json

ASSET = 'croisement02-shrub-57'
DIGEST = 'd8281fe58969556b760a81cc7feb4b5bf8b9b2e149355e53d8620e313a9fa253'


def selected_workspace(out, asset, catalog_path):
    if asset != ASSET:
        return None
    receipt_path = out / 'restart2-fence/shrub57-approved-selection-v1.json'
    if not receipt_path.exists():
        return None
    receipt = json.loads(receipt_path.read_text())
    group = next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id'] == asset)
    if receipt['group'] != group:
        raise ValueError('Approved shrub57 ownership changed')
    for file, digest in receipt['files'].items():
        if sha(Path(file)) != digest:
            raise ValueError('Approved shrub57 evidence changed: ' + file)
    worker = Path(receipt['workspace'])
    companion=worker/'inspection/source-authority-companion-v1.json'
    if companion.exists():
        if sha(companion)!='8d2ed04245a515097fb892aeed6c1edc8044870419387d633111cc04c78c4f54':raise ValueError('Approved shrub57 source companion changed')
        source=json.loads(companion.read_text())
        if source['model_sha256']!=DIGEST or sha(Path(source['parent_authority']))!=source['parent_authority_sha256']:
            raise ValueError('Approved shrub57 source companion authority changed')
        for filename,digest in source['files'].items():
            if sha(Path(filename))!=digest:raise ValueError('Approved shrub57 source metadata changed: '+filename)
    if sha(worker / 'model.blend') != DIGEST:
        raise ValueError('Approved shrub57 model changed')
    return worker


def register(out, catalog_path):
    frozen = out / 'restart2-fence/shrub57-geometry-review-v1'
    proof = out / 'restart2-fence/shrub57-sign-bend-v9'
    old = out / 'understory-round-9/assets' / ASSET
    worker = out / 'restart2-fence/shrub57-approved-package-v1/assets' / ASSET
    approval = frozen / 'user-geometry-approval.json'
    decision = json.loads(approval.read_text())
    assert decision['decision'] == 'approved' and decision['model_sha256'] == DIGEST
    assert sha(frozen / 'model.blend') == DIGEST
    worker.mkdir(parents=True, exist_ok=False)
    for name in ['model.blend', 'validation.json']:
        shutil.copy2(frozen / name, worker / name)
    for name in ['workspace.json', 'source-masks.json']:
        shutil.copy2(old / name, worker / name)
    (worker / 'inspection').mkdir()
    files = [worker / n for n in ['model.blend', 'validation.json', 'workspace.json', 'source-masks.json']]
    files += [approval, frozen / 'gallery/evidence.json', proof / 'root-geometry-readiness.json',
              proof / 'report.json', proof / 'actual-eight.png', proof / 'ground-parity-proof/report.json',
              proof / 'native-physical-comparison/report.json', proof / 'painted-shadow-appearance.json']
    group = next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id'] == ASSET)
    receipt = dict(status='Explicitly user-approved geometry; appearance completion and publication separate',
                   workspace=str(worker), model_sha256=DIGEST, group=group,
                   files={str(p): sha(p) for p in files},
                   original_workspace=str(old),
                   limitations=decision['limitations'],
                   metadata_scope='Workspace/source masks retain original ownership. No old geometry audit or renders are presented as current.')
    write_json(out / 'restart2-fence/shrub57-approved-selection-v1.json', receipt)
    write_json(worker / 'inspection/approved-geometry-authority.json', receipt)
    assert selected_workspace(out, ASSET, catalog_path) == worker
    print(worker)


if __name__ == '__main__':
    from catalog import OUT, reviewed_catalog
    register(OUT, reviewed_catalog())

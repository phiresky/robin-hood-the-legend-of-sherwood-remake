"""Expose the separately reviewed post cap without inheriting fence approval."""
import json
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
ASSET='croisement02-east-upright-rail-fence-95'


def selected_workspace(out,asset,catalog):
    target=out/'missing-fence-candidates/post-cap95-selection.json'
    if asset!=ASSET or not target.exists():return None
    receipt=json.loads(target.read_text())
    if receipt['approval']!='pending':raise ValueError('Cap selector cannot inherit approval')
    group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if group!=receipt['group']:raise ValueError('Cap source ownership changed')
    for path,digest in receipt['files'].items():
        if sha(Path(path))!=digest:raise ValueError('Cap evidence changed: '+path)
    worker=Path(receipt['worker'])
    if sha(worker/'model.blend')!=receipt['model_sha256']:raise ValueError('Cap geometry changed')
    return worker


def expose():
    from catalog import OUT,reviewed_catalog
    directory=OUT/'missing-fence-candidates/post-cap95-v1';worker=directory/'assets'/ASSET
    digest=sha(worker/'model.blend');review=json.loads((worker/'inspection/visual-review.json').read_text());preserved=json.loads((worker/'inspection/reopened-preservation.json').read_text())
    if preserved['status']!='PASS' or preserved['model_sha256']!=digest or review['model_sha256']!=digest:raise ValueError('Stale cap review')
    for key,path in [('actual8_sha256',worker/'inspection/actual-materials/sheet.png'),('solid8_sha256',worker/'modified/solid.png'),('native_comparison_sha256',directory/'source-comparison/comparison.png'),('preservation_sha256',worker/'inspection/reopened-preservation.json')]:
        if review[key]!=sha(path):raise ValueError('Stale cap review image')
    group=next(g for g in json.loads(reviewed_catalog().read_text())['groups'] if g['id']==ASSET)
    receipt=directory/'root-review.json'
    write_json(receipt,dict(status='PASS: scoped cap geometry and inferred source roles',model_sha256=digest,reviewer='root coordinator',approval='pending user review',source_roles=dict(cap95=7,existing95=1,foliage75=28,ground=15),notes=['Native cap comparison and saved actual eight views independently inspected.','Original fence remains exact. No wood plate over green gaps.','One cap boundary pixel remains partial alpha32/255; source role correction is inferred.']))
    paths=[p for p in worker.rglob('*') if p.is_file()]
    paths += [receipt,directory/'source-review.json',directory/'source-comparison/evidence.json',directory/'source-comparison/comparison.png',directory/'domain-431.png']
    paths += list((OUT/'missing-fence-candidates/boundary-roles95-v1').glob('*'))
    original=json.loads((worker/'inspection/preservation.json').read_text())
    for path,expected in original['protected_files'].items():
        if sha(Path(path))!=expected:raise ValueError('Approved fence changed')
        paths.append(Path(path))
    for path,expected in json.loads((worker/'modified/views.json').read_text()).get('source_mask_evidence',{}).items():
        if sha(Path(path))!=expected:raise ValueError('Cap source changed')
        paths.append(Path(path))
    target=OUT/'missing-fence-candidates/post-cap95-selection.json'
    if target.exists():raise FileExistsError(target)
    write_json(target,dict(asset_id=ASSET,worker=str(worker),group=group,model_sha256=digest,approval='pending',files={str(p):sha(p) for p in paths if p.is_file()},scope='Fresh additive cap geometry; previous approved95 retained unchanged. Source roles inferred, canonical full-scene source integration separate.'))
    assert selected_workspace(OUT,ASSET,reviewed_catalog())==worker
    print(target)

if __name__=='__main__':expose()


def geometry_review_ready(out,asset,model_sha256):
    """Read supplemental current-neighbor readiness without editing frozen cap evidence."""
    if asset!=ASSET:return None
    path=out/'restart2-fence/fence95-root-ready-v1.json'
    if not path.exists():return None
    receipt=json.loads(path.read_text())
    if receipt['model_sha256']!=model_sha256:raise ValueError('Fence95 supplemental geometry review is stale')
    if not receipt['status'].startswith('PASS') or not receipt['ready_for_geometry_review']:return None
    for name,digest in receipt['files'].items():
        if sha(Path(name))!=digest:raise ValueError('Fence95 supplemental evidence changed: '+name)
    return receipt

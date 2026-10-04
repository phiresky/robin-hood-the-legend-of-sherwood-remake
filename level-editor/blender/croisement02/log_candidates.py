"""Select additive log geometry without inheriting earlier geometry approvals."""
import argparse
import json
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
from catalog_schema import source_for_part
ASSET='croisement02-southwest-log-pile'
PARTS=['building-130','building-134','building-135']


def validate_worker(worker):
    model=sha(worker/'model.blend')
    records={name:json.loads((worker/name).read_text()) for name in (
        'inspection/visual-review.json','inspection/refinement.json','inspection/saved-model-audit.json',
        'inspection/reopened-preservation.json','inspection/source-coverage/report.json',
        'inspection/source-coverage/residual-review.json','inspection/approved-baseline-comparison/comparison.json')}
    if any(r['model_sha256']!=model for r in records.values()):raise ValueError('Stale log evidence')
    review=records['inspection/visual-review.json']
    if not all(review.get(k) for k in ('ready_for_geometry_review','all_eight_actual_views_inspected','all_eight_solid_views_inspected','all_eight_source_only_views_inspected')):raise ValueError('Incomplete log visual review')
    if review['actual_materials_sha256']!=sha(worker/'inspection/actual-materials/sheet.png') or review['comparison_sha256']!=sha(worker/'inspection/approved-baseline-comparison/comparison.png'):raise ValueError('Log review images changed')
    if any(r['nonmanifold_edges'] or r['degenerate_faces'] for r in records['inspection/refinement.json']['new_members']):raise ValueError('Invalid added log topology')
    if records['inspection/saved-model-audit.json']['status']!='PASS' or records['inspection/reopened-preservation.json']['status']!='PASS' or json.loads((worker/'validation.json').read_text())['status']!='PASS':raise ValueError('Log validation failed')
    preserved=json.loads((worker/'inspection/preservation.json').read_text())
    if not preserved['existing_mesh_appearance_identical'] or records['inspection/reopened-preservation.json']['existing_objects_unchanged']!=preserved['existing_objects']:raise ValueError('Log baseline appearance changed')
    for path,digest in preserved['protected_files'].items():
        if sha(Path(path))!=digest:raise ValueError('Approved log baseline changed')
    residual=records['inspection/source-coverage/residual-review.json']
    if sum(r['pixels'] for r in residual['categories'])!=records['inspection/source-coverage/report.json']['missing_pixels']:raise ValueError('Incomplete residual accounting')


def selected_workspace(out,asset,catalog):
    if asset!=ASSET:return None
    path=out/'southwest-log-revision/selection.json'
    if not path.exists():return None
    receipt=json.loads(path.read_text())
    if receipt['asset_id']!=asset or receipt['approval']!='pending':raise ValueError('Invalid log selection')
    group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if group!=receipt['group'] or sorted(source_for_part(p) for p in group['parts'])!=PARTS:raise ValueError('Log ownership changed')
    for path,digest in receipt['files'].items():
        if sha(Path(path))!=digest:raise ValueError('Log candidate evidence changed: '+path)
    worker=Path(receipt['worker']);validate_worker(worker)
    return worker


def expose(worker):
    from catalog import OUT,reviewed_catalog
    validate_worker(worker)
    cfg=json.loads((worker/'workspace.json').read_text())
    if cfg['asset_id']!=ASSET or sorted(cfg['part_ids'])!=PARTS:raise ValueError('Unexpected log scope')
    group=next(g for g in json.loads(reviewed_catalog().read_text())['groups'] if g['id']==ASSET)
    paths=[worker/'model.blend',worker/'baseline.blend',worker/'workspace.json',worker/'source-masks.json',worker/'validation.json']
    for name in ('inspection','recipe','reference','mask-reference','input','modified','projection'):
        paths.extend(p for p in sorted((worker/name).rglob('*')) if p.is_file())
    preservation=json.loads((worker/'inspection/preservation.json').read_text())
    paths.extend(Path(p) for p in preservation['protected_files'])
    for path,digest in json.loads((worker/'modified/views.json').read_text()).get('source_mask_evidence',{}).items():
        if sha(Path(path))!=digest:raise ValueError('Log source evidence changed')
        paths.append(Path(path))
    target=OUT/'southwest-log-revision/selection.json'
    if target.exists():raise FileExistsError('Archive previous log selection explicitly before replacement')
    write_json(target,dict(asset_id=ASSET,worker=str(worker),model_sha256=sha(worker/'model.blend'),group=group,approval='pending',files={str(p):sha(p) for p in paths},rationale='Coordinator reviewed additive geometry and preserved baseline appearance. Residual true wood outlines and ambiguous plant boundaries remain documented. New hidden surfaces require fill after geometry approval. No prior approval is inherited.'))
    if selected_workspace(OUT,ASSET,reviewed_catalog())!=worker:raise ValueError('Log selection failed')
    print(target)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('worker',type=Path);expose(p.parse_args().worker.resolve())

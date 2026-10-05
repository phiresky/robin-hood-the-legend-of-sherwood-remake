"""Select reviewed rock completion with pinned source and joint evidence."""
import argparse
import json
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
from catalog_schema import source_for_part
ASSET='croisement02-northwest-rock-outcrop'
PARTS=['building-035','building-036','building-133']


def validate_worker(worker):
    model=sha(worker/'model.blend')
    read=lambda name:json.loads((worker/name).read_text())
    review=read('inspection/visual-review.json')
    audit=read('inspection/saved-model-audit.json')
    coverage=read('inspection/source-domain-coverage/report.json')
    if any(r['model_sha256']!=model for r in (review,audit,coverage)):
        raise ValueError('Stale northwest rock evidence')
    if not review['ready_for_geometry_review'] or audit['status']!='PASS' or read('validation.json')['status']!='PASS':
        raise ValueError('Northwest rock review is incomplete')
    if sorted(r['source_node'] for r in audit['objects'])!=PARTS or coverage['source_coverage']<.99:
        raise ValueError('Northwest rock scope or source coverage differs')
    for key,name in [('source_coverage_sha256','source-domain-coverage/report.json'),('native_front_depth_sha256','native-front-depth.json'),('middle_front_depth_sha256','native-front-depth-036.json'),('joint_neighbourhood_sha256','joint-neighbourhood.json')]:
        if review[key]!=sha(worker/'inspection'/name):raise ValueError('Rock review evidence changed: '+name)
    complete=Path(review['complete_volume_evidence'])
    if sha(complete)!=review['complete_volume_evidence_sha256']:raise ValueError('Complete volume evidence changed')
    proof=json.loads(complete.read_text())
    if proof['model_sha256']!=model:raise ValueError('Stale complete volume model')
    for key,path in [('fixed_manifest_sha256',worker/'modified/views.json'),('fitted_manifest_sha256',complete.parent/'views.json'),('actual_sheet_sha256',complete.parent/'textured-sheet.png'),('solid_sheet_sha256',complete.parent/'solid-sheet.png')]:
        if proof[key]!=sha(path):raise ValueError('Complete volume packet changed')
    joint=read('inspection/joint-neighbourhood.json')
    if joint['model_sha256']!=model or sha(Path(joint['evidence']))!=joint['evidence_sha256'] or sha(Path(joint['sheet']))!=joint['sheet_sha256']:
        raise ValueError('Rock joint changed')
    for row in json.loads(Path(joint['evidence']).read_text())['workers']:
        if sha(Path(row['path'])/'model.blend')!=row['model_sha256']:raise ValueError('Joint neighbour changed')


def selected_workspace(out,asset,catalog):
    from approved_ledge_selection import selected_workspace as approved_ledge_workspace
    approved=approved_ledge_workspace(out,asset,catalog)
    if approved is not None:return approved
    if asset!=ASSET:return None
    path=out/'northwest-rock-source-revision/selection.json'
    if not path.exists():return None
    receipt=json.loads(path.read_text())
    group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if receipt['asset_id']!=asset or receipt['approval']!='pending' or group!=receipt['group'] or sorted(source_for_part(p) for p in group['parts'])!=PARTS:
        raise ValueError('Northwest rock selection scope changed')
    for path,digest in receipt['files'].items():
        if sha(Path(path))!=digest:raise ValueError('Northwest rock evidence changed: '+path)
    worker=Path(receipt['worker']);validate_worker(worker)
    return worker


def expose(worker):
    from catalog import OUT,reviewed_catalog
    validate_worker(worker)
    cfg=json.loads((worker/'workspace.json').read_text())
    if cfg['asset_id']!=ASSET or sorted(cfg['part_ids'])!=PARTS:raise ValueError('Unexpected rock worker scope')
    group=next(g for g in json.loads(reviewed_catalog().read_text())['groups'] if g['id']==ASSET)
    paths=[worker/n for n in ('model.blend','baseline.blend','workspace.json','source-masks.json','validation.json')]
    for folder in ('inspection','recipe','reference','mask-reference','input','modified','projection'):
        paths.extend(p for p in (worker/folder).rglob('*') if p.is_file())
    for path,digest in json.loads((worker/'modified/views.json').read_text()).get('source_mask_evidence',{}).items():
        if sha(Path(path))!=digest:raise ValueError('Rock source evidence changed')
        paths.append(Path(path))
    joint=json.loads((worker/'inspection/joint-neighbourhood.json').read_text())
    paths.extend(p for p in Path(joint['evidence']).parent.iterdir() if p.is_file())
    target=OUT/'northwest-rock-source-revision/selection.json'
    if target.exists():raise FileExistsError('Archive prior rock selection before replacing it')
    write_json(target,dict(asset_id=ASSET,worker=str(worker),model_sha256=sha(worker/'model.blend'),group=group,approval='pending',files={str(p):sha(p) for p in paths},limitations=['Faint old/new rock joins remain visible.','Hidden and out-of-map surfaces remain gray pending approved texture fill.','Joint ground is diagnostic Z0; full terrain integration remains separate.'],rationale='New rounded out-of-map completion with independently reviewed native source coverage, complete-volume views, and support-corrected shrub54 joint. No previous approval is inherited.'))
    if selected_workspace(OUT,ASSET,reviewed_catalog())!=worker:raise ValueError('Rock selection failed')
    print(target)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('worker',type=Path);expose(parser.parse_args().worker.resolve())

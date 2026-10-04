"""Pin an independently reviewed shrub revision without modifying prior workers."""
import json
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json


def selected_workspace(out,asset,catalog_path):
    northwest=asset=='croisement02-northwest-boundary-shrub-54'
    western=asset in ('croisement02-shrub-57','croisement02-shrub-60')
    forest_round={'croisement02-shrub-62':11,'croisement02-shrub-63':13,'croisement02-shrub-64':8}.get(asset)
    clump_round={f'croisement02-shrub-{i}':r for i,r in [(77,16),(78,12),(83,18),(84,12),(74,15),(85,15),(86,15),(87,20),(88,15),(89,17),(90,15),(93,22)]}.get(asset)
    if asset=='croisement02-canopy-fringe-22':clump_round=21
    if asset=='croisement02-shrub-76':clump_round=1
    refit=out/(f'understory-round-{clump_round}/assets' if clump_round else f'understory-round-{forest_round}/assets' if forest_round else 'understory-round-7/assets' if northwest else 'understory-round-9/assets' if western else 'understory-round-2/assets')/asset
    if asset=='croisement02-shrub-76':refit=out/'understory-candidates/native76-clumps-v1/assets'/asset
    candidate=refit/'inspection/shrub-candidate.json'
    proof=refit/'inspection'/('support-evidence.json' if northwest or western or forest_round or clump_round else 'refit-evidence.json')
    if candidate.exists() and proof.exists():
        receipt=json.loads(candidate.read_text())
        group=next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id']==asset)
        if receipt.get('group')!=group or not group.get('authored_scenery') or 'native_foliage_mask' not in group:
            raise ValueError('Refitted shrub ownership changed')
        model_hash=sha(refit/'model.blend')
        review=json.loads((refit/'inspection/visual-review.json').read_text())
        audit=json.loads((refit/'inspection/saved-model-audit.json').read_text())
        if (receipt['model_sha256']!=model_hash or review['model_sha256']!=model_hash or audit['model_sha256']!=model_hash
                or audit['status']!='PASS' or not review['ready_for_geometry_review']
                or review['sheet_sha256']!=sha(refit/'inspection/actual-materials/sheet.png')):
            raise ValueError('Refitted shrub review changed')
        if (northwest or western or forest_round or clump_round) and review.get('support_evidence_sha256')!=sha(proof):raise ValueError('Shrub support evidence changed')
        if (forest_round and asset!='croisement02-shrub-64') or asset=='croisement02-shrub-83':
            fill=refit/'inspection/inferred-fill-evidence.json'
            if review.get('inferred_fill_evidence_sha256')!=sha(fill):raise ValueError('Forest inferred leaf fill evidence changed')
        if asset=='croisement02-shrub-93':
            budget=refit/'inspection/render-budget-evidence.json'
            proof_budget=json.loads(budget.read_text())
            if (review.get('render_budget_evidence_sha256')!=sha(budget) or proof_budget['model_sha256']!=model_hash
                    or proof_budget['current_transparent_bounces']<256 or sha(Path(proof_budget['current_report']))!=proof_budget['current_report_sha256']):
                raise ValueError('Dense shrub transparent traversal proof changed')
        coverage=json.loads((refit/'inspection/source-coverage/report.json').read_text())
        bounds=json.loads((refit/'inspection/actual-materials/opacity-bounds.json').read_text())
        if coverage['model_sha256']!=model_hash or bounds['model_sha256']!=model_hash or coverage['intersection_over_union']<.95 or min(c['depth_width_ratio'] for c in bounds['crowns'])<1:
            raise ValueError('Registered shrub source or physical bounds changed')
        joint_path=refit/'inspection/joint-neighbourhood.json'
        if sha(joint_path)!=review['joint_neighbourhood_sha256']:raise ValueError('Refitted shrub joint receipt changed')
        joint=json.loads(joint_path.read_text())
        if sha(Path(joint['evidence']))!=joint['evidence_sha256'] or sha(Path(joint['sheet']))!=joint['sheet_sha256']:
            raise ValueError('Refitted shrub joint evidence changed')
        for row in json.loads(Path(joint['evidence']).read_text())['workers']:
            if sha(Path(row['path'])/'model.blend')!=row['model_sha256']:raise ValueError('Refitted shrub neighbour changed')
        return refit
    path=out/'understory-candidates/west-bank-v7/selection.json'
    if not path.exists():path=out/'understory-candidates/west-bank-v6/selection.json'
    if not path.exists():path=out/'understory-candidates/west-bank-v5/selection.json'
    if asset!='croisement02-west-shrub-bank' or not path.exists():return None
    receipt=json.loads(path.read_text())
    group=next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id']==asset)
    if group!=receipt['group']:raise ValueError('Reviewed shrub ownership changed')
    for filename,digest in receipt['files'].items():
        if sha(Path(filename))!=digest:raise ValueError('Reviewed shrub evidence changed: '+filename)
    return Path(receipt['workspace'])


def register(out,catalog_path,round_number=5):
    if round_number not in (5,6,7):raise ValueError('Only reviewed bank revisions5/6/7 are supported')
    asset='croisement02-west-shrub-bank'
    worker=out/f'understory-round-{round_number}/assets'/asset
    previous=out/f'understory-round-{round_number-1}/assets'/asset
    directory=out/f'understory-candidates/west-bank-v{round_number}'
    revision=json.loads((directory/'revision.json').read_text())
    if sha(previous/'model.blend')!=revision['previous_model_sha256']:raise ValueError('Previous selected worker changed')
    model_hash=sha(worker/'model.blend')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
    coverage=json.loads((worker/'inspection/source-coverage/report.json').read_text())
    bounds=json.loads((worker/'inspection/actual-materials/opacity-bounds.json').read_text())
    validation=json.loads((worker/'validation.json').read_text())
    if (not review.get('ready_for_geometry_review') or audit['status']!='PASS' or validation['status']!='PASS'
            or any(r['model_sha256']!=model_hash for r in (review,audit,coverage,bounds))
            or coverage['intersection_over_union']<.95 or min(c['depth_width_ratio'] for c in bounds['crowns'])<1
            or review['sheet_sha256']!=sha(worker/'inspection/actual-materials/sheet.png')):
        raise ValueError('Shrub revision is not independently reviewable')
    joint_path=worker/'inspection/joint-neighbourhood.json'
    joint=json.loads(joint_path.read_text())
    if (review['joint_neighbourhood_sha256']!=sha(joint_path) or joint['model_sha256']!=model_hash
            or sha(Path(joint['evidence']))!=joint['evidence_sha256'] or sha(Path(joint['sheet']))!=joint['sheet_sha256']):
        raise ValueError('Joint evidence is stale')
    evidence=json.loads(Path(joint['evidence']).read_text())
    dependencies=[Path(r['path'])/'model.blend' for r in evidence['workers']]
    if any(sha(path)!=row['model_sha256'] for path,row in zip(dependencies,evidence['workers'])):raise ValueError('Joint neighbour changed')
    preservation=Path(review['preservation_evidence'])
    if sha(preservation)!=review['preservation_evidence_sha256']:raise ValueError('Native RGB preservation evidence changed')
    group=next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id']==asset)
    if group['native_foliage_masks']!=[56,61] or group['parts']!=[dict(node='foliage-west-shrub-bank',name='Western foreground shrubs',foliage_domain_mask=413)]:raise ValueError('Bank source ownership changed')
    files=[previous/'model.blend',worker/'model.blend',worker/'workspace.json',worker/'validation.json',
        *[worker/'inspection'/name for name in ('visual-review.json','saved-model-audit.json','refinement.json','source-coverage/report.json','actual-materials/sheet.png','actual-materials/opacity-bounds.json')],
        joint_path,Path(joint['evidence']),Path(joint['sheet']),preservation,directory/'revision.json',*dependencies]
    if round_number>=6:
        proof=worker/'inspection/support-evidence.json'
        if review.get('support_evidence_sha256')!=sha(proof):raise ValueError('Bank support evidence changed')
        files.append(proof)
    if round_number==7:
        proof=worker/'inspection/inferred-fill-evidence.json'
        if review.get('inferred_fill_evidence_sha256')!=sha(proof):raise ValueError('Bank inferred fill evidence changed')
        files.append(proof)
    write_json(directory/'selection.json',dict(workspace=str(worker),model_sha256=model_hash,group=group,files={str(p):sha(p) for p in files},status='Reviewed geometry revision candidate; no user approval or publication implied'))
    write_json(worker/'inspection/shrub-candidate.json',dict(model_sha256=model_hash,status='Reviewed replacement geometry candidate; native domain413 unchanged'))
    print(worker)


if __name__=='__main__':
    from catalog import OUT,reviewed_catalog
    register(OUT,reviewed_catalog())

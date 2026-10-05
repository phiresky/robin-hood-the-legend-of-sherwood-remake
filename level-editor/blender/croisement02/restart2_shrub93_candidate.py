"""Select a reviewed additive boundary revision while preserving old approvals."""
import json,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from evidence_io import sha,write_json
ASSET='croisement02-shrub-93'


def selected_workspace(out,asset,catalog_path):
    receipt=out/'restart2-vegetation/shrub93-selection.json'
    if asset!=ASSET or not receipt.exists():return None
    row=json.loads(receipt.read_text())
    group=next(g for g in json.loads(catalog_path.read_text())['groups']if g['id']==asset)
    if row['group']!=group:raise ValueError('Boundary foliage ownership changed')
    for path,digest in row['files'].items():
        if sha(Path(path))!=digest:raise ValueError('Boundary foliage evidence changed: '+path)
    return Path(row['workspace'])


def register(out,catalog_path):
    destination=out/'restart2-vegetation/shrub93-selection.json'
    if destination.exists():raise FileExistsError(destination)
    worker=out/'restart2-vegetation/shrub93-package-v3/assets'/ASSET
    root_path=worker/'inspection/root-review.json';root=json.loads(root_path.read_text());digest=sha(worker/'model.blend')
    if root['status']!='PASS' or root['model_sha256']!=digest:raise ValueError('Independent root review missing')
    visual=json.loads((worker/'inspection/visual-review.json').read_text())
    coverage=json.loads((worker/'inspection/source-coverage/report.json').read_text())
    bounds=json.loads((worker/'inspection/actual-materials/opacity-bounds.json').read_text())
    preservation=json.loads((worker/'inspection/package-preservation.json').read_text())
    audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
    validation=json.loads((worker/'validation.json').read_text())
    if (any(r['model_sha256']!=digest for r in [visual,coverage,bounds,preservation,audit])
            or audit['status']!='PASS' or validation['status']!='PASS'
            or visual['sheet_sha256']!=sha(worker/'inspection/actual-materials/sheet.png')
            or not visual['ready_for_geometry_review'] or coverage['intersection_over_union']<.98
            or len(bounds['crowns'])!=3 or min(r['depth_width_ratio']for r in bounds['crowns'])<1
            or len(bounds['attached_boundary_fragments'])!=1
            or not preservation['exact_geometry_uv_material_preservation']):raise ValueError('Boundary foliage validation failed')
    joint=json.loads((worker/'inspection/joint-neighbourhood.json').read_text())
    rays=Path(root['first_hit_evidence']);hits=json.loads(rays.read_text())
    if not any(r['model_sha256']==digest for r in hits['workers']):raise ValueError('Boundary first-hit model differs')
    if hits['joint_evidence_sha256']!=sha(Path(joint['evidence'])):raise ValueError('Boundary first-hit neighbours differ')
    if len(hits['records'])!=25 or any(r['asset']!=ASSET for r in hits['records']):raise ValueError('Boundary first-hit coverage failed')
    files=[worker/'model.blend',worker/'workspace.json',worker/'source-masks.json',worker/'validation.json',root_path,rays]
    files+=list((worker/'inspection').rglob('*.json'))+list((worker/'inspection').rglob('*.png'))
    joint_folder=Path(joint['evidence']).parent;files+=list(joint_folder.glob('*.json'))+list(joint_folder.glob('*.png'))
    for row in json.loads(Path(joint['evidence']).read_text())['workers']:
        model=Path(row['path'])/'model.blend';assert sha(model)==row['model_sha256'];files.append(model)
    raw=Path(preservation['source_candidate']);files+=[raw/'model.blend',raw/'preservation.json']
    for path,h in preservation['protected'].items():assert sha(Path(path))==h;files.append(Path(path))
    inventory=Path(json.loads((worker/'source-masks.json').read_text())['mask_inventory']);files.append(inventory)
    for row in json.loads(inventory.read_text())['masks']:
        if row['index']in [503,6003]:files.append((inventory.parent/row['png']).resolve())
    group=next(g for g in json.loads(catalog_path.read_text())['groups']if g['id']==ASSET)
    write_json(destination,dict(workspace=str(worker),group=group,model_sha256=digest,files={str(p):sha(p)for p in files},user_approval=False,scope='New additive25 inferred boundary geometry; old user approvals do not transfer.'))
    assert selected_workspace(out,ASSET,catalog_path)==worker
    print(destination)

if __name__=='__main__':
    from catalog import OUT,reviewed_catalog
    register(OUT,reviewed_catalog())

"""Select the explicitly approved ledge with original source authority and new guards."""
import json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
ASSET='croisement02-northwest-rock-outcrop'
MODEL='1acb5d328a7035255b7a381adc7e1e6550da625d0b72b725b8dfa4be3560c0d8'

def selected_workspace(out,asset,catalog):
    if asset!=ASSET:return None
    package=out/'restart3-nwledge-approved/assets'/asset
    authority=package/'inspection/approved-geometry-authority.json'
    if not authority.exists():return None
    data=json.loads(authority.read_text());group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if data['group']!=group or data['model_sha256']!=MODEL:raise ValueError('Approved ledge ownership changed')
    for name,digest in data['files'].items():
        if sha(Path(name))!=digest:raise ValueError('Approved ledge evidence changed: '+name)
    decision=json.loads(Path(data['geometry_decision']).read_text())
    if decision['decision']!='approved' or decision['scope']!='geometry' or decision['model_sha256']!=MODEL:raise ValueError('Missing exact ledge geometry approval')
    if sha(package/'model.blend')!=MODEL:raise ValueError('Approved ledge model changed')
    return package

def package():
    from catalog import OUT,reviewed_catalog
    old=Path(json.loads((OUT/'northwest-rock-source-revision/selection.json').read_text())['worker'])
    current=OUT/'restart2-bank321/northwest-ledge26-v2'
    root=json.loads((current/'root-review.json').read_text())
    proof=json.loads((current/'reopened-review/preservation.json').read_text())
    if sha(current/'worker.blend')!=MODEL or proof['status']!='PASS' or proof['model_sha256']!=MODEL:raise ValueError('Ledge preservation changed')
    assert proof['old_uvs_materials_packed_images_exact'] and proof['other_geometry_and_rock_appearance_exact'] and proof['maximum_supported_base_change']==0
    decisions=json.loads((OUT/'user-feedback.json').read_text())['records']
    decision=next(r for r in decisions if r['asset_id']==ASSET and r['model_sha256']==MODEL and r['decision']=='approved' and r['scope']=='geometry')
    archive=Path(decision['archive']);decision_path=archive/'decision.json'
    package=OUT/'restart3-nwledge-approved/assets'/ASSET
    package.mkdir(parents=True,exist_ok=False);(package/'inspection').mkdir()
    shutil.copy2(current/'worker.blend',package/'model.blend')
    for name in ['workspace.json','source-masks.json']:shutil.copy2(old/name,package/name)
    paths=[package/'model.blend',package/'workspace.json',package/'source-masks.json',old/'modified/views.json',decision_path,current/'root-review.json',current/'reopened-review/preservation.json',current/'full-source-audit.json',current/'classification-final.json',current/'support.json',current/'validation.json',current/'geometry-gallery/evidence.json']
    write_json(package/'validation.json',dict(status='PASS',model_sha256=MODEL,scope='Exact reopened geometry preservation, independent source/contact review and explicit user geometry decision; original workspace metadata supplies source ownership only',preservation=str(current/'reopened-review/preservation.json'),preservation_sha256=sha(current/'reopened-review/preservation.json')))
    paths.append(package/'validation.json')
    write_json(package/'inspection/approved-geometry-authority.json',dict(status='Explicitly user-approved geometry; later texture completion remains separate',workspace=str(package),model_sha256=MODEL,
        group=next(g for g in json.loads(reviewed_catalog().read_text())['groups'] if g['id']==ASSET),geometry_decision=str(decision_path),original_workspace=str(old),
        source_authority=dict(manifest=str(old/'modified/views.json'),manifest_sha256=sha(old/'modified/views.json'),purpose='Original source/camera/ownership metadata only; this is not a render of the derivative'),
        files={str(p):sha(p) for p in paths},metadata_scope='Unchanged workspace/source ownership copied. No old geometry audit or render packet is presented as current.',limitations=['Three residual native center samples are partial contact edges; no full pixel-center coverage claim.','New ledge native overlay and geometry are explicitly approved; unrelated terrain/state completion remains separate.']))
    assert selected_workspace(OUT,ASSET,reviewed_catalog())==package
    print(package)
if __name__=='__main__':package()

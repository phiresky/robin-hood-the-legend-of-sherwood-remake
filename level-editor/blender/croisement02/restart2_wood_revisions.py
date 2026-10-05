"""Strict intermediate tree32/38 wood selections; unfinished crowns stay on hold."""
import argparse,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement/blender')]
from evidence_io import sha,write_json
from catalog_schema import source_for_part


def read(path):return json.loads(path.read_text())
def require(value,message):
    if not value:raise ValueError(message)


def selected_workspace(out,mask,catalog_path):
    if mask not in (32,38):return None
    path=out/'restart2-wood/selections'/f'tree-{mask}.json'
    if not path.exists():return None
    record=read(path);worker=Path(record['worker']);digest=sha(worker/'model.blend');inspect=worker/'inspection'
    require(record['kind']=='scoped-lower-wood-working-base' and record['mask']==mask,'Wrong intermediate selection kind')
    require(record['approval']=='pending' and record['whole_tree_status']=='HOLD: legacy crown refinement and texture completion','Intermediate selection must not imply final approval')
    require(worker.name==record['asset_id']==f'croisement02-tree-{mask:02d}' and digest==record['model_sha256'],'Wrong/stale wood worker')
    group=next(g for g in read(catalog_path)['groups'] if g['id']==worker.name)
    require({source_for_part(p) for p in group['parts']}==set(record['part_ids']),'Wood source scope changed')
    for file,expected in record['files'].items():require(sha(Path(file))==expected,'Bound wood evidence changed: '+file)
    require(record['independent_review']['reviewer']=='root independent' and record['independent_review']['status']=='PASS scoped lower wood','Independent scoped review required')
    proof=read(inspect/'root-preservation.json');coverage=read(inspect/'source-coverage/report.json');local=read(inspect/'root-source-coverage/report.json');audit=read(inspect/'saved-model-audit.json');bounds=read(inspect/'actual-materials/opacity-bounds.json')
    for row in (proof,coverage,local,audit,bounds):require(row['model_sha256']==digest,'Stale worker verification')
    require(read(worker/'validation.json')['status']=='PASS' and audit['status']=='PASS','Invalid worker')
    require(proof['preserved'] and proof['previous_meshes']==proof['current_meshes'] and len(proof['current_meshes'])>=190,'Protected scene appearance changed')
    require(any(f'Tree {mask:02d} / Crown' in key for key in proof['current_meshes']),'Own crown missing from preservation')
    require(sha(Path(proof['previous_worker'])/'model.blend')==proof['previous_model_sha256'],'Original worker changed')
    require(coverage['intersection_over_union']>=.95 and all(local[k]>=.95 for k in ('source_coverage','interface_source_coverage','root_source_coverage')),'Source coverage failed')
    if mask==38:
        ground=read(inspect/'root-source-coverage-ground/report.json');require(ground['model_sha256']==digest and ground['extension_coverage']>=.95,'Ground contact source coverage failed')
    require(bounds['crowns'] and min(c['depth_width_ratio'] for c in bounds['crowns'])>=1,'Frozen crown depth regressed')
    joint=read(inspect/'joint-neighbourhood.json');require(joint['model_sha256']==digest and sha(Path(joint['evidence']))==joint['evidence_sha256'],'Joint evidence changed')
    for row in read(Path(joint['evidence']))['workers']:require(sha(Path(row['path'])/'model.blend')==row['model_sha256'],'Frozen joint neighbour changed')
    return worker


def bind(mask,worker,reviewed,review_text):
    from catalog import OUT,reviewed_catalog
    digest=sha(worker/'model.blend');cfg=read(worker/'workspace.json');paths=set(reviewed)
    for name in ('model.blend','workspace.json','source-masks.json','validation.json','mask-reference/native-hashes.json'):paths.add(worker/name)
    for name in ('root-preservation.json','source-coverage/report.json','root-source-coverage/report.json','saved-model-audit.json','actual-materials/opacity-bounds.json','joint-neighbourhood.json','wood-self-review.json'):paths.add(worker/'inspection'/name)
    if mask==38:paths.add(worker/'inspection/root-source-coverage-ground/report.json')
    packet=read(worker/'source-masks.json');paths.add(Path(packet['mask_inventory']))
    for file,expected in read(worker/'mask-reference/native-hashes.json').items():require(sha(Path(file))==expected,'Source masks changed');paths.add(Path(file))
    path=OUT/'restart2-wood/selections'/f'tree-{mask}.json';path.parent.mkdir(parents=True,exist_ok=True);require(not path.exists(),'Preserve frozen selection receipt')
    write_json(path,dict(kind='scoped-lower-wood-working-base',mask=mask,asset_id=worker.name,worker=str(worker),model_sha256=digest,part_ids=cfg['part_ids'],approval='pending',whole_tree_status='HOLD: legacy crown refinement and texture completion',independent_review=dict(reviewer='root independent',status='PASS scoped lower wood',exact_review_text=review_text),files={str(p.resolve()):sha(p) for p in sorted(paths)},limitations=['Intermediate base for further refinement, not whole-tree readiness or user approval.','Preserved crown striping and unknown rear wood texture remain separate unfinished work.']))
    require(selected_workspace(OUT,mask,reviewed_catalog())==worker,'Selection validation failed')
    return path

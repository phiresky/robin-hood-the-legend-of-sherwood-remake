"""Snapshot selected scene workers and exact-compatible approved textures privately."""
import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT,reviewed_catalog,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from approved_texture_stage import select


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('output',type=Path);args=parser.parse_args()
    output=args.output.resolve()
    if output.exists():raise FileExistsError(output)
    output.mkdir(parents=True);shutil.copy2(reviewed_catalog(),output/'catalog.json')
    catalog=json.loads((output/'catalog.json').read_text())
    if len(catalog['groups'])!=93:raise ValueError('Expected93 catalog snapshot')
    records=[];models={};trees={};scenery={}
    for group in catalog['groups']:
        if group.get('state_only'):
            records.append(dict(id=group['id'],role='state-only metadata; not visible in base scene',parts=group['parts']));continue
        worker=tree_workspace(group['wood_mask']) if 'wood_mask' in group else scenery_workspace(group['id'])
        frozen=output/'workers'/group['id'];(frozen/'inspection').mkdir(parents=True)
        before=sha(worker/'model.blend')
        for name in ['model.blend','inspection/saved-model-audit.json','inspection/feedback-revision-1.json']:
            if (worker/name).exists():
                subprocess.run(['cp','--reflink=auto','--preserve=mode,timestamps',str(worker/name),str(frozen/name)],check=True)
        if sha(frozen/'model.blend')!=before or sha(worker/'model.blend')!=before:raise ValueError('Worker changed while freezing '+group['id'])
        models[group['id']]=frozen/'model.blend'
        if 'wood_mask' in group:trees[str(group['wood_mask'])]=str(frozen)
        else:scenery[group['id']]=str(frozen)
        records.append(dict(id=group['id'],worker=str(worker),frozen=str(frozen),model_sha256=before,has_saved_audit=(frozen/'inspection/saved-model-audit.json').exists()))
    decisions_path=OUT/'texture-review/decisions.json';decisions=json.loads(decisions_path.read_text());latest={r['asset_id']:r for r in decisions['decisions']}
    accepted=[];omitted=[]
    for asset,row in latest.items():
        if row.get('scope')!='texture' or row.get('decision')!='approved':continue
        proof=Path(row['evidence_paths']['model']).parent/'reopened-preservation.json'
        evidence=json.loads(proof.read_text())
        if asset not in models or evidence['model_sha256']!=sha(models[asset]):
            omitted.append(dict(asset=asset,reason='Approved texture base differs from selected geometry',approved_base_sha256=evidence['model_sha256'],selected_sha256=sha(models[asset]) if asset in models else None));continue
        accepted.append(row)
    write_json(output/'texture-decisions.json',dict(decisions=accepted))
    if accepted:select(output/'texture-decisions.json',models)
    ground=OUT/'ground-receiver-review-v5'
    subprocess.run(['cp','--reflink=auto','--preserve=mode,timestamps',str(ground/'model.blend'),str(output/'ground.blend')],check=True)
    write_json(output/'snapshot.json',dict(catalog_sha256=sha(output/'catalog.json'),catalog_groups=len(catalog['groups']),workers=records,trees=trees,scenery=scenery,approved_texture_count=len(accepted),texture_omissions=omitted,original_decisions_sha256=sha(decisions_path),ground_worker=str(ground),ground_model_sha256=sha(output/'ground.blend'),approval='Private pending-geometry scene review; not publication'))
    print('Frozen',len(models),'workers,',len(accepted),'compatible approved textures;',len(omitted),'texture mismatches')


if __name__=='__main__':main()

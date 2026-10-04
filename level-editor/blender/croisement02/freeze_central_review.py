"""Bind already completed independent review to a central foliage proposal."""
import argparse
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from central_candidates import validate_record,DOMAINS


def freeze(index,version,root_review):
    batch=OUT/f'understory-candidates/native-{index}-{version}';worker=batch/'assets'/f'croisement02-shrub-{index:02}'
    inspection=worker/'inspection';joint=OUT/f'ground-plant-candidates/joint-v2/native-shrub-{index}-{version}'
    root=json.loads(root_review.read_text());model=sha(worker/'model.blend');sheet=inspection/'actual-materials/sheet.png'
    if not root['ready'] or root['model_sha256']!=model:raise ValueError('Missing independent geometry readiness')
    for path,digest in root['files'].items():
        if sha(Path(path))!=digest:raise ValueError('Independent review evidence changed')
    joint_path=inspection/'joint-neighbourhood.json'
    write_json(joint_path,dict(model_sha256=model,evidence=str(joint/'evidence.json'),evidence_sha256=sha(joint/'evidence.json'),sheet=str(joint/'sheet.png'),sheet_sha256=sha(joint/'sheet.png'),label='Exact source and four oblique views with selected terrain/neighbour geometry'))
    preservation=batch/'source-rgb-validation.json'
    write_json(inspection/'visual-review.json',dict(model_sha256=model,sheet_sha256=sha(sheet),ready_for_geometry_review=True,user_approval=None,review=root['review'],root_review=str(root_review),root_review_sha256=sha(root_review),preservation_evidence=str(preservation),preservation_evidence_sha256=sha(preservation),joint_neighbourhood_sha256=sha(joint_path)))
    group=next(g for g in json.loads((batch/'catalog.json').read_text())['groups'] if g['id']==worker.name)
    files=[worker/'model.blend',worker/'workspace.json',worker/'validation.json',sheet,inspection/'refinement.json',inspection/'saved-model-audit.json',inspection/'source-coverage/report.json',inspection/'actual-materials/opacity-bounds.json',inspection/'visual-review.json',joint_path,preservation,root_review,batch/f'domain-{DOMAINS[index]}.png',batch/f'shrub-{index:02}/partition.json',batch/f'shrub-{index:02}/support.json',batch/'scope-derivation.json']
    files.extend(joint/n for n in ('sheet.png','evidence.json','native-scale-context.png','source-overlay.png'))
    for name in ('support-preservation.json','prior-source-render-comparison.json','appearance-preservation.json','appearance-reopen.json'):
        path=inspection/name
        if path.exists():files.append(path)
    files.extend(batch/f'shrub-{index:02}'/n for n in ('complete-source.png','observed-source.png'))
    source_roles=batch/'source-role-review.json'
    if source_roles.exists():
        files.append(source_roles);roles=json.loads(source_roles.read_text());authority=Path(roles['authority'])
        if sha(authority)!=roles['authority_sha256']:raise ValueError('Mixed source authority changed')
        files.append(authority)
        review=json.loads(authority.read_text());evidence=Path(review['evidence']);original_domain=Path(roles['record']['domain_path'])
        if sha(evidence)!=review['evidence_sha256'] or sha(original_domain)!=roles['record']['domain_sha256']:raise ValueError('Mixed source split evidence changed')
        files.extend([evidence,original_domain])
        for path_key,hash_key in [('root_review','root_review_sha256'),('prior_authority','prior_authority_sha256')]:
            if path_key in review:
                path=Path(review[path_key])
                if sha(path)!=review[hash_key]:raise ValueError('Mixed source review dependency changed')
                files.append(path)
    for dependency in json.loads((joint/'evidence.json').read_text())['inputs']:
        path=Path(dependency['workspace'])/'model.blend'
        if sha(path)!=dependency['model_sha256']:raise ValueError('Joint dependency changed')
        files.append(path)
    record=dict(native_mask=index,domain=DOMAINS[index],group=group,workspace=str(worker),batch=str(batch),model_sha256=model,user_approval=None,files={str(p):sha(p) for p in files})
    validate_record(record,group);write_json(batch/'reviewed-proposal.json',record)
    print(batch/'reviewed-proposal.json')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('mask',type=int);parser.add_argument('--version',required=True);parser.add_argument('--root-review',type=Path,required=True)
    args=parser.parse_args();freeze(args.mask,args.version,args.root_review.resolve())

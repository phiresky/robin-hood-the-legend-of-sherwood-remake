"""Strict new shed east return geometry selection; old approvals never transfer."""
import hashlib,json
from pathlib import Path

def sha(p):
    with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def selected_workspace(out,asset,catalog):
    if asset!='croisement02-woodcutters-shed':return None
    receipt=out/'restart4-source-gaps/shed-ready-candidate-v2.json'
    if not receipt.exists():return None
    if sha(receipt)!='5b15026d1e951185bc207757f389d60c54aafec70ebee3e6c942df9738266d4d':
        raise ValueError('Shed east return reviewed receipt changed')
    data=json.loads(receipt.read_text())
    for filename,digest in data['files'].items():
        if sha(filename)!=digest:raise ValueError('Shed east return evidence changed: '+filename)
    group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if len(group['parts'])!=2 or sorted(p.get('obstacle', -1) for p in group['parts'])!=[138,139]:
        raise ValueError('Shed east return canonical ownership changed')
    worker=Path(data['worker']);model=sha(worker/'model.blend')
    if model!=data['model_sha256']:raise ValueError('Shed east return model changed')
    for filename in ['validation.json','inspection/saved-model-audit.json']:
        check=json.loads((worker/filename).read_text())
        if check['status']!='PASS':raise ValueError('Shed east return technical validation failed')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    root=json.loads((worker/'inspection/root-review.json').read_text())
    if review['model_sha256']!=model or root['model_sha256']!=model or not review['ready_for_geometry_review'] or not root['status'].startswith('PASS'):
        raise ValueError('Shed east return scoped visual review changed')
    return worker

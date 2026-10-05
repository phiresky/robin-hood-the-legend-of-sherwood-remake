"""Strict new kindling geometry selection; old approvals never transfer."""
import hashlib,json
from pathlib import Path

def sha(p):
    with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def selected_workspace(out,asset,catalog):
    if asset!='croisement02-southwest-kindling-bundle':return None
    receipt=out/'restart3-kindling/ready-candidate-v1.json'
    if not receipt.exists():return None
    if sha(receipt)!='e432f4d7a1a42056c2cdc632eabe52b197e39438b1e578d963ddcdf21d43718c':
        raise ValueError('Kindling reviewed receipt changed')
    data=json.loads(receipt.read_text())
    for filename,digest in data['files'].items():
        if sha(filename)!=digest:raise ValueError('Kindling evidence changed: '+filename)
    group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if len(group['parts'])!=1 or group['parts'][0].get('obstacle')!=129:
        raise ValueError('Kindling canonical ownership changed')
    worker=Path(data['worker']);model=sha(worker/'model.blend')
    if model!=data['model_sha256']:raise ValueError('Kindling model changed')
    for filename in ['validation.json','inspection/saved-model-audit.json']:
        check=json.loads((worker/filename).read_text())
        if check['status']!='PASS':raise ValueError('Kindling technical validation failed')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    root=json.loads((worker/'inspection/root-review.json').read_text())
    if review['model_sha256']!=model or root['model_sha256']!=model or not review['ready_for_geometry_review'] or not root['status'].startswith('PASS'):
        raise ValueError('Kindling scoped visual review changed')
    return worker

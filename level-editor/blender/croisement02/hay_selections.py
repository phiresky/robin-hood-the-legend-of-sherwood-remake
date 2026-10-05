"""Strict new haystack geometry selection; old approvals never transfer."""
import hashlib,json
from pathlib import Path

def sha(p):
    with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def selected_workspace(out,asset,catalog):
    if asset!='croisement02-south-field-haystack':return None
    receipt=out/'restart3-hay/ready-candidate-v1.json'
    if not receipt.exists():return None
    if sha(receipt)!='b348e2b83d5327eed2ec2c23da3ecd57b0db3bfbf5745391b13506209b8f13b2':
        raise ValueError('Haystack reviewed receipt changed')
    data=json.loads(receipt.read_text())
    for filename,digest in data['files'].items():
        if sha(filename)!=digest:raise ValueError('Haystack evidence changed: '+filename)
    group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if len(group['parts'])!=2 or sorted(p.get('obstacle', -1) for p in group['parts'])!=[140,141]:
        raise ValueError('Haystack canonical ownership changed')
    worker=Path(data['worker']);model=sha(worker/'model.blend')
    if model!=data['model_sha256']:raise ValueError('Haystack model changed')
    for filename in ['validation.json','inspection/saved-model-audit.json']:
        check=json.loads((worker/filename).read_text())
        if check['status']!='PASS':raise ValueError('Haystack technical validation failed')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    root=json.loads((worker/'inspection/root-review.json').read_text())
    if review['model_sha256']!=model or root['model_sha256']!=model or not review['ready_for_geometry_review'] or not root['status'].startswith('PASS'):
        raise ValueError('Haystack scoped visual review changed')
    return worker

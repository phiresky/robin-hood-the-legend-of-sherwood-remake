"""Strict new logging stump geometry selection; old approvals never transfer."""
import hashlib,json
from pathlib import Path

def sha(p):
    with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def selected_workspace(out,asset,catalog):
    if asset!='croisement02-logging-clearing-stumps':return None
    receipt=out/'restart4-source-gaps/stump-ready-candidate-v2.json'
    if not receipt.exists():return None
    if sha(receipt)!='756425d03f1a29b01cc1b8817871efba17862cf89872be872b17071b2d74b4bf':
        raise ValueError('Logging stump reviewed receipt changed')
    data=json.loads(receipt.read_text())
    for filename,digest in data['files'].items():
        if sha(filename)!=digest:raise ValueError('Logging stump evidence changed: '+filename)
    group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if len(group['parts'])!=2 or sorted(p.get('obstacle', -1) for p in group['parts'])!=[26,27]:
        raise ValueError('Logging stump canonical ownership changed')
    worker=Path(data['worker']);model=sha(worker/'model.blend')
    if model!=data['model_sha256']:raise ValueError('Logging stump model changed')
    for filename in ['validation.json','inspection/saved-model-audit.json']:
        check=json.loads((worker/filename).read_text())
        if check['status']!='PASS':raise ValueError('Logging stump technical validation failed')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    root=json.loads((worker/'inspection/root-review.json').read_text())
    if review['model_sha256']!=model or root['model_sha256']!=model or not review['ready_for_geometry_review'] or not root['status'].startswith('PASS'):
        raise ValueError('Logging stump scoped visual review changed')
    return worker

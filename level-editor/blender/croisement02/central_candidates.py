"""Hash-bound selection of reviewed central native foliage candidates."""
import json
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha
DOMAINS={73:470,92:471,67:472,68:473,69:474,70:475,71:476,72:477,79:490,80:491,82:492}


def validate_record(record,group):
    index=record['native_mask'];worker=Path(record['workspace']);node=f'foliage-shrub-{index:03}'
    if record['group']!=group or group.get('native_foliage_mask')!=index or record['domain']!=DOMAINS[index]:raise ValueError('Central foliage ownership changed')
    if record.get('user_approval') is not None:raise ValueError('New geometry cannot inherit approval')
    for filename,digest in record['files'].items():
        if sha(Path(filename))!=digest:raise ValueError('Central foliage evidence changed: '+filename)
    scope_path=worker.parents[1]/'scope-derivation.json'
    if scope_path.exists():
        if str(scope_path) not in record['files']:raise ValueError('Scoped source derivation not frozen')
        scope=json.loads(scope_path.read_text())
        if scope['targets']!=[group['id']] or sha(Path(scope['source']))!=scope['source_sha256']:
            raise ValueError('Scoped source scene or target changed')
    model=sha(worker/'model.blend')
    audit=json.loads((worker/'inspection/saved-model-audit.json').read_text());coverage=json.loads((worker/'inspection/source-coverage/report.json').read_text())
    review=json.loads((worker/'inspection/visual-review.json').read_text());geometry=json.loads((worker/'inspection/refinement.json').read_text())['crown']
    if model!=record['model_sha256'] or any(r['model_sha256']!=model for r in (audit,coverage,review)):raise ValueError('Central model binding changed')
    if audit['status']!='PASS' or not review['ready_for_geometry_review'] or coverage['intersection_over_union']<.95:raise ValueError('Central geometry checks failed')
    if geometry['geometry_version'] not in ('native-shrub-leaf-volume-v2','native-conifer-leaf-volume-v1'):raise ValueError('Unreviewed geometry recipe')
    if geometry['opacity_bounds']['depth_width_ratio']<1 or [r['source_node'] for r in audit['objects']]!=[node]:raise ValueError('Central physical bounds or scope failed')
    support=geometry['support']
    if sha(Path(support['bank_worker'])/'model.blend')!=support['bank_model_sha256'] or abs(support['minimum_elevation']-support['support_point'][2]-.5)>.002:raise ValueError('Central support evidence changed')
    return worker


def selected_workspace(out,asset,catalog_path):
    path=out/'central-foliage-integration/selection.json'
    if not path.exists():return None
    record=json.loads(path.read_text())['records'].get(asset)
    if record is None:return None
    group=next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id']==asset)
    return validate_record(record,group)

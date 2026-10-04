"""Strict selection of reviewed native grass and fern hypotheses."""
import json
import math
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha


def validate_record(record,group):
    if record['group']!=group or group.get('native_ground_plant_mask')!=record['native_mask']:
        raise ValueError('Ground-plant scope changed')
    if record.get('user_approval') is not None:raise ValueError('Registration cannot inherit approval')
    index=record['native_mask'];node=f'foliage-ground-plant-{index:03}'
    if not 111<=index<=123 or record['domain']!=440+index-111 or [p['node'] for p in group['parts']]!=[node]:
        raise ValueError('Unexpected native plant domain')
    for name,digest in record['files'].items():
        if sha(Path(name))!=digest:raise ValueError('Ground-plant evidence changed: '+name)
    worker=Path(record['workspace']);model=sha(worker/'model.blend')
    audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
    coverage=json.loads((worker/'inspection/source-coverage/report.json').read_text())
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    geometry=json.loads((worker/'inspection/refinement.json').read_text())['crown']
    if (model!=record['model_sha256'] or any(d['model_sha256']!=model for d in [audit,coverage,review])
            or audit['status']!='PASS' or not review['ready_for_geometry_review']
            or coverage['intersection_over_union']<.95
            or geometry['geometry_version']!='native-rooted-ground-plants-v4'
            or abs(geometry['ground_z']-(36/math.cos(math.radians(35)) if index>=117 else 0))>.001
            or abs(geometry['minimum_z']-geometry['ground_z']-.05)>.002
            or [r['source_node'] for r in audit['objects']]!=[node]):
        raise ValueError('Ground-plant review, geometry or support failed')
    return worker


def selected_workspace(out,asset,catalog_path):
    if not asset.startswith('croisement02-ground-plant-'):return None
    path=out/'ground-plant-integration/selection.json'
    if not path.exists():return None
    receipt=json.loads(path.read_text());record=receipt['records'].get(asset)
    if record is None:raise ValueError('Unregistered ground plant')
    group=next(g for g in json.loads(catalog_path.read_text())['groups'] if g['id']==asset)
    return validate_record(record,group)

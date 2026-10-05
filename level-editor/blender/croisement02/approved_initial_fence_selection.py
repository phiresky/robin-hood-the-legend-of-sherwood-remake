"""Select the explicitly approved initial fence with original source authority and new guards."""
import json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha
ASSET='croisement02-south-field-wattle-fence'
MODEL='46579f5398d1495b13e8d8433fa1a0687be447e025cd9ba745f5d7251076c5a0'

def selected_workspace(out,asset,catalog):
    if asset!=ASSET:return None
    package=out/'restart4-initial-fence-approved/assets'/asset
    authority=package/'inspection/approved-geometry-authority.json'
    if not authority.exists():return None
    data=json.loads(authority.read_text());group=next(g for g in json.loads(catalog.read_text())['groups'] if g['id']==asset)
    if data['group']!=group or data['model_sha256']!=MODEL:raise ValueError('Approved initial fence ownership changed')
    for name,digest in data['files'].items():
        if sha(Path(name))!=digest:raise ValueError('Approved initial fence evidence changed: '+name)
    decision=json.loads(Path(data['geometry_decision']).read_text())
    if decision['decision']!='approved' or decision['scope']!='geometry' or decision['model_sha256']!=MODEL:raise ValueError('Missing exact initial fence geometry approval')
    if sha(package/'model.blend')!=MODEL:raise ValueError('Approved initial fence model changed')
    return package


"""Explicit ownership after reviewing all six source-artwork survey sheets."""
import json
import hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'

def reviewed_catalog():
    revised=OUT/'ownership-revision/catalog.json'
    return revised if revised.exists() else OUT/'catalog.json'

def bank_workspace(asset):
    """Select the reviewed bank only while its scope and evidence remain current."""
    if asset!='croisement02-north-woodland-bank':return None
    worker=OUT/'terrain-bank-candidate/assets'/asset
    path=worker/'inspection/bank-candidate.json'
    if not path.exists():return None
    receipt=json.loads(path.read_text())
    group=next(g for g in json.loads(reviewed_catalog().read_text())['groups'] if g['id']==asset)
    parts=sorted(f"building-{p['obstacle']:03}" for p in group['parts'] if 'obstacle' in p)
    expected=[f'building-{i:03}' for i in range(5)]
    if (parts!=expected or len(group['parts'])!=5 or receipt.get('part_ids')!=expected
            or receipt.get('asset_id')!=asset):
        raise ValueError('Bank candidate ownership differs from native parts0–4')
    for relative,expected_hash in receipt['files'].items():
        file=OUT/'terrain-bank-candidate'/relative
        if not file.is_file() or hashlib.sha256(file.read_bytes()).hexdigest()!=expected_hash:
            raise ValueError('Bank candidate evidence changed: '+relative)
    model_hash=hashlib.sha256((worker/'model.blend').read_bytes()).hexdigest()
    audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    if (model_hash!=receipt['model_sha256'] or audit['status']!='PASS'
            or audit['model_sha256']!=model_hash or review['model_sha256']!=model_hash
            or sorted(r['source_node'] for r in audit['objects'])!=expected):
        raise ValueError('Bank candidate model/audit/review binding differs')
    return worker


def scenery_workspace(asset):
    from prop_completion_candidates import selected_workspace as prop_workspace
    completed_prop=prop_workspace(OUT,asset,reviewed_catalog())
    if completed_prop is not None:return completed_prop
    from rock_candidates import selected_workspace as rock_workspace
    rock=rock_workspace(OUT,asset,reviewed_catalog())
    if rock is not None:return rock
    from central_candidates import selected_workspace as central_workspace
    central=central_workspace(OUT,asset,reviewed_catalog())
    if central is not None:return central
    from ground_plant_candidates import selected_workspace as plant_workspace
    ground_plant=plant_workspace(OUT,asset,reviewed_catalog())
    if ground_plant is not None:return ground_plant
    from log_candidates import selected_workspace as log_workspace
    revised_log=log_workspace(OUT,asset,reviewed_catalog())
    if revised_log is not None:return revised_log
    from shrub_candidates import selected_workspace as shrub_workspace
    revised_shrub=shrub_workspace(OUT,asset,reviewed_catalog())
    if revised_shrub is not None:return revised_shrub
    from fence_candidates import selected_workspace as selected_fence_workspace
    fence=selected_fence_workspace(OUT,asset,reviewed_catalog())
    if fence is not None:return fence
    from stem_candidates import selected_workspace
    joined_stem=selected_workspace(OUT,asset,reviewed_catalog())
    if joined_stem is not None:return joined_stem
    bank=bank_workspace(asset)
    if bank is not None:return bank
    western=OUT/'understory-round-4/assets'/asset
    if (western/'inspection/shrub-candidate.json').exists():return western
    shrub=OUT/'understory-round-1/assets'/asset
    if (shrub/'inspection/shrub-candidate.json').exists():return shrub
    authored=OUT/'authored-stems-round-1/assets'/asset
    if (authored/'inspection/authored-integration.json').exists():return authored
    source_revised=OUT/'scenery-round-3/assets'/asset
    if (source_revised/'inspection/rock-ownership-revision.json').exists():return source_revised
    replacement=OUT/'scenery-round-2/assets'/asset
    if any((replacement/'inspection'/receipt).exists() for receipt in ('feedback-revision-1.json','relief-revision.json')):
        return replacement
    return OUT/'scenery-round-1/assets'/asset

def tree_workspace(mask):
    from canopy_candidates import selected_workspace
    cleanup=selected_workspace(OUT,mask,reviewed_catalog())
    if cleanup is not None:return cleanup
    from restart2_wood_revisions import selected_workspace as revised_wood_workspace
    revised_wood=revised_wood_workspace(OUT,mask,reviewed_catalog())
    if revised_wood is not None:return revised_wood
    asset=f'croisement02-tree-{mask:02}'
    completed=OUT/'forest-v4-round-3/assets'/asset
    if (completed/'inspection/northern-cap-revision.json').exists():return completed
    replacement=OUT/'forest-v4-round-2/assets'/asset
    if (replacement/'inspection/source-domain-revision.json').exists():return replacement
    return OUT/'forest-v4-round-1/assets'/asset

# Each key is the native wood mask. Parts are observed pieces of the same tree.
TREES={0:[44,45],1:[46,47,48,64],2:[49],3:[50,51,144],4:[52,53],5:[54],6:[55,56,57],
7:[58,60,61,62],8:[59],10:[63,65,66],11:[67,68,69],12:[70,71,72],13:[73,74],14:[75,76],
15:[33],16:[99,100,102,105],17:[103,104],18:[101],19:[125,126],20:[106,107],23:[34],
24:[124],25:[120],26:[118],27:[121],28:[119],29:[117],30:[116],31:[77,78,79],
32:[80,81,82],33:[83],34:[84],35:[90,91],36:[88,89],37:[92,93],38:[94],39:[95,96,97],
40:[98],41:[85,86,87],42:[114,115],43:[108,109],45:[110,111],46:[112,113],47:[127,128]}
GROUPS=[
('north-woodland-bank','North Woodland Bank',[0,1,2,3,4,37]),
('east-stone-wall-and-gate','East Stone Wall and Gate',[5,6,7,8,9,10,11]),
('southeast-stone-wall-and-gate','Southeast Stone Wall and Gate',[12,13,14,15,16,17]),
('east-rail-fence','East Rail Fence',[18,24]),
('south-field-wattle-fence','South Field Wattle Fence',[19,20]),
('southwest-path-wattle-fence','Southwest Path Wattle Fence',[21]),
('southwest-field-wattle-fence','Southwest Field Wattle Fence',[22,23]),
('northeast-oak-root-bank','Northeast Oak Root Bank',[25]),
('logging-clearing-stumps','Logging Clearing Stumps',[26,27]),
('logging-clearing-log','Logging Clearing Fallen Log',[28]),
('north-firewood-stack','North Firewood Stack',[29,30]),
('north-kindling-bundle','North Kindling Bundle',[31,32]),
('northwest-rock-outcrop','Northwest Rock Outcrop',[35,36]),
('west-rock-outcrop','West Rock Outcrop',[38,39,40,41,42]),
('southwest-rock-outcrop','Southwest Rock Outcrop',[43,131,133,136,137]),
('southwest-stumps','Southwest Stumps',[122,123]),
('southwest-kindling-bundle','Southwest Kindling Bundle',[129]),
('southwest-log-pile','Southwest Log Pile',[130,134,135]),
('west-root-bank','West Root Bank',[132]),
('woodcutters-shed','Woodcutters Shed',[138,139]),
('south-field-haystack','South Field Haystack',[140,141]),
('central-covered-state','Central Covered State',[142,143]),
('south-fence-applied-state','South Fence Applied State',[145]),
('north-applied-state-assembly','North Applied State Assembly',[146,147,148,149])]

def main():
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());groups=[]
    for slug,name,parts in GROUPS:
        groups.append(dict(id='croisement02-'+slug,name=name,parts=[dict(obstacle=i,name=f'{name} part {i:03}') for i in parts]))
    for mask,parts in TREES.items():
        x,y=level['masks'][mask]['box_top_left'];region=('North' if y<300 else 'South' if y>700 else 'Central')+('west' if x<600 else 'east' if x>1200 else '')
        name=f'{region} Tree {mask:02}'
        if mask==42:name='South Field Oak'
        groups.append(dict(id=f'croisement02-tree-{mask:02}',name=name,wood_mask=mask,
                           parts=[dict(obstacle=i,name=f'{name} wood {i:03}') for i in parts]))
    ids=[p['obstacle'] for g in groups for p in g['parts']]
    assert sorted(ids)==list(range(150)), 'Missing or duplicate native source ownership'
    data=dict(version=1,map='Croisement02',groups=groups,
              review_notes='Trunks and their visible limbs are grouped using the six source survey sheets and wood masks. Terrain is separate. Mission-only graphics remain inventoried by source-states/layers.json; this catalog does not claim they are modeled.')
    (OUT/'catalog.json').write_text(json.dumps(data,indent=2)+'\n')
    print('Catalog:',len(groups),'groups,',len(ids),'native parts')

if __name__=='__main__':main()

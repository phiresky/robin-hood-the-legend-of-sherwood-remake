"""Source-survey grouping for Croisement03; candidates remain unapproved."""
import hashlib
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement03-refinement'
TREES={0:[0,1],1:[2,3,4,5],2:[6],3:[7],4:[8,9,10],5:[11,12,14],6:[13],7:[15,16,17],8:[18],9:[19],10:[20],11:[21,22,23],12:[24,25,47],13:[31,32,48],14:[26,27],15:[28,29,30],18:[33,34],19:[39],20:[37,38],21:[40],22:[41],23:[43],24:[42,44],25:[46],26:[45,50,51]}
GROUPS=[
 ('northwest-shrub-rocks','Northwest Shrub Rocks',[35,36],[104,105,106]),
 ('southwest-firewood-stack','Southwest Firewood Stack',[49],[114]),
 ('northwest-high-rock-outcrop','Northwest High Rock Outcrop',[52,53,54],[96,108,109]),
 ('northwest-path-boulders','Northwest Path Boulders',[55,56,57],[94,95]),
 ('north-path-boulders','North Path Boulders',[58,59,60,61],[87,88]),
 ('west-cliff-outcrop','West Cliff Outcrop',[62,85,86,100],[86,126]),
 ('west-path-rocks','West Path Rocks',[63,64,65,66,67],[89,90,91]),
 ('southwest-rock-outcrop','Southwest Rock Outcrop',[68,69],[92,93,103]),
 ('central-shrub-boulder','Central Shrub Boulder',[70],[97]),
 ('south-stream-stones','South Stream Stones',[71,72,73,76,77,83],[99,100,101,102,115]),
 ('east-tree-rocks','East Tree Rocks',[74,75],[98]),
 ('woodland-small-rocks','Woodland Small Rocks',[78,79,80,81,82,84],[107]),
 ('southeast-stone-wall','Southeast Stone Wall',[87,88,89,90,91,92,93],[113]),
 ('west-cliff-path','West Cliff Path',[94,95,96,97],[]),
 ('central-removable-barrier','Central Removable Barrier',[98,99],[128,129]),
 ('north-path-applied-obstruction','North Path Applied Obstruction',[101],[127]),
 ('south-path-applied-obstruction','South Path Applied Obstruction',[102,103,104,105],[]),
]
def main():
    groups=[]
    for mask,parts in TREES.items():
        name=f'Woodland Tree {mask:02}'
        groups.append(dict(id=f'croisement03-tree-{mask:02}',name=name,wood_mask=mask,parts=[dict(obstacle=n,name=f'Wood {n:03}') for n in parts]))
    for slug,name,parts,masks in GROUPS:
        groups.append(dict(id='croisement03-'+slug,name=name,native_mask_candidates=masks,parts=[dict(obstacle=n,name=f'{name} part {n:03}') for n in parts]))
    actual=[p['obstacle'] for g in groups for p in g['parts']]
    assert sorted(actual)==list(range(106)), 'Missing or duplicated native part'
    data=dict(version=1,map='Croisement03',groups=groups,review_notes='Logical source survey grouping only. Native mask candidates are not blanket RGB ownership. Foreground exclusions, physical geometry, authored bridge, mask-only vegetation, moving water, and mission states remain unfinished.')
    (OUT/'catalog.json').write_text(json.dumps(data,indent=2)+'\n')
    sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    (OUT/'grouping-review.json').write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(OUT/'catalog.json'),inventory_sha256=sha(OUT/'inventory/inventory.json'),evidence='All four obstacle source sheets, all nine individual-mask survey sheets and both grayscale occlusion layers inspected. Trees are grouped by same-stem artwork; root/cliff, barrier and applied obstacle groups retain separate ownership. No geometry or texture approval.'),indent=2)+'\n')
    print(json.dumps(dict(groups=len(groups),native_parts=len(actual))))
if __name__=='__main__': main()

"""Build a source-complete Sherwood grouping candidate from inspected world geometry.

No live library mutation: the candidate is reviewed before the publication step.
Named component ownership separates built access structures from their trees.
"""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import sys

EDITOR = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(EDITOR/'refinement/blender'))
from catalog_schema import parse_catalog, source_for_part


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def build(geometry, output):
    geometry, output = Path(geometry), Path(output)
    output.mkdir(parents=True, exist_ok=True)
    original = json.loads((EDITOR/'refinement/catalogs/sherwood.json').read_text())
    index = parse_catalog(original)
    records = json.loads(geometry.read_text())
    parts = {source_for_part(p): p for g in original['groups'] for p in g['parts']}
    groups, claimed, reasons = [], set(), {}

    def add(slug, name, numbers, reason):
        sources = [f'building-{n:03}' for n in numbers]
        assert not claimed.intersection(sources), (slug, claimed.intersection(sources))
        claimed.update(sources)
        identity = 'sherwood-'+slug
        groups.append(dict(id=identity, name=name, parts=[dict(parts[s]) for s in sources]))
        reasons[identity] = reason

    add('west-camp-hut', 'West camp hut', [0,1,2,3,4], 'One complete hut: roof, walls, doorway, chimney and attached side shelter.')
    add('north-camp-hut', 'North camp hut', [8,9,10], 'Join the main hut and its directly attached supply shelter into one building.')
    add('camp-dining-shelter', 'Camp dining shelter', [11,12,15,16], 'The canopy, table and two fitted benches form one dining assembly.')
    add('camp-preparation-table-set', 'Camp preparation table set', [14,17,18], 'The preparation table and its two adjacent benches form one furniture set; loose props remain separate.')
    add('central-oak', 'Central oak', [32,33,48,49,52], 'Join the trunk, upper trunk, cradle limb, buttress and spreading root. Keep the house and access platform separate.')
    add('broad-woodland-oak', 'Broad woodland oak', [36,38], 'The detached upper limb belongs to this oak, confirmed by its mesh recipe and world position.')
    add('ladder-oak', 'Ladder oak', [24], 'Tree geometry only; its rope ladder belongs to the separate access platform.')
    add('ladder-oak-platform', 'Ladder oak platform and ladder', [86,92], 'Join the ring platform, its landing and attached rope-ladder geometry.')
    add('central-oak-treehouse', 'Central oak treehouse', [102,103], 'House walls and roof only; remove the oak cradle limb and put access rails and ladders with the platform.')
    add('central-oak-platform', 'Central oak platform and landings', [88,91,96,99], 'Join the ring platform, extension, two stair landings, braces, rails and connecting ladders.')
    add('west-treehouse-oak', 'West treehouse oak', [29], 'Keep the tree separate from its house and built access assembly.')
    add('west-treehouse', 'West treehouse', [89,100,105,106,107], 'Join the house, side roof, lower and upper platforms and connecting ladder, as requested in the grouping review. The supporting oak remains separate.')
    add('central-west-treehouse-oak', 'Central west treehouse oak', [30], 'Separate the oak from the built treehouse and porch.')
    add('central-west-treehouse', 'Central west treehouse', [104], 'Complete house structure, separated from its supporting oak.')
    add('central-west-treehouse-platform', 'Central west treehouse porch', [87], 'Keep the built porch separately usable from the tree and house.')
    add('west-border-treehouse-oak', 'West border treehouse oak', [46], 'Separate the border tree from its built house and porch.')
    add('west-border-treehouse', 'West border treehouse', [90,119,120], 'Join the border house, porch and porch barrel, as requested in the grouping review. The supporting oak remains separate.')
    add('central-suspension-bridge', 'Central suspension bridge', [93], 'This span connects the central west house to the central oak; it is not the westernmost bridge.')
    add('long-suspension-bridge', 'Long suspension bridge', [94], 'Keep the complete span reusable separately from the platforms at either end.')
    add('west-suspension-bridge', 'West suspension bridge', [95], 'This western span joins the west treehouse to the ladder oak; correct its former east-bridge label.')
    add('west-camp-lookout-shelter', 'West camp lookout shelter', [83,84,85], 'Join the shelter body, side and adjacent entry landing; these are one camp structure in the source art.')
    add('southwest-work-shelter', 'Southwest work shelter', [108,109], 'Join the small southwest roof and the work surface underneath; neither belongs to the central treehouse.')
    add('southwest-worktable', 'Southwest worktable', [110], 'Correct the bed label: this is the separate outdoor work surface south of the shelter.')
    add('north-woodland-shelter', 'North woodland shelter', [122,123,124], 'Join the partly obscured woodland shelter, attached front structure and small adjacent vessel into one dressed asset.')
    add('west-camp-rock-cluster', 'West camp rock cluster', [57,58], 'Two touching rocks beside the southwest camp shelter form a continuous outcrop.')
    add('central-oak-rock-outcrop', 'Central oak rock outcrop', [59,60], 'Join the touching rock slabs immediately east of the central oak.')
    add('south-riverbank-rock-outcrop', 'South riverbank rock outcrop', [61,62,63,64,65], 'Join the continuous five-rock outcrop at the southern river crossing.')
    add('east-riverbank-rock-outcrop', 'East riverbank rock outcrop', [75,76,77], 'Join the overlapping rock masses on the east bank; retain the separated nearby stones individually.')
    add('ladder-oak-rock-cluster', 'Ladder oak rock cluster', [79,80,81], 'Three touching rocks behind the ladder oak form one small cluster.')
    for group in original['groups']:
        remaining = [dict(p) for p in group['parts'] if source_for_part(p) not in claimed]
        if remaining:
            groups.append({**group, 'parts':remaining})
            claimed.update(source_for_part(p) for p in remaining)
            reasons[group['id']] = 'Retain this independent complete asset; proximity or canopy overlap alone is not a reason to merge.'
    assert claimed == index.sources
    by_id = {g['id']:g for g in groups}
    component_owners = {}
    for number, home, destination, predicate in [
        (24,'sherwood-ladder-oak','sherwood-ladder-oak-platform',lambda n: n.startswith('Ladder oak') and 'tapered fluted trunk' not in n),
        (102,'sherwood-central-oak-treehouse','sherwood-central-oak-platform',lambda n: not n.startswith('Central hut - wall')),
    ]:
        source = f'building-{number:03}'
        owned = [o for o in records if o['source']==source]
        for identity, selected in [(home,[o['name'] for o in owned if not predicate(o['name'])]),
                                   (destination,[o['name'] for o in owned if predicate(o['name'])])]:
            assert selected
            part = dict(parts[source], components=sorted(selected))
            existing = next((p for p in by_id[identity]['parts'] if source_for_part(p)==source),None)
            if existing: existing.update(part)
            else: by_id[identity]['parts'].append(part)
            component_owners.update({name:identity for name in selected})
    canonical = {source_for_part(p):g['id'] for g in groups for p in g['parts']}
    canonical.update({'building-024':'sherwood-ladder-oak','building-102':'sherwood-central-oak-treehouse'})
    catalog = {**original,'version':2,'groups':groups,'canonical_owners':canonical,
               'grouping_review':{'status':'pending','basis':'Full-scene world geometry and original-art audit',
                                  'source_catalog_sha256':sha(EDITOR/'refinement/catalogs/sherwood.json')}}
    parsed = parse_catalog(catalog, expected_sources=index.sources)
    checked=[]
    assignments={}
    for obj in records:
        source=obj['source']
        if source=='ground':assignments[obj['name']]='sherwood-terrain';continue
        component=obj['name'] if source in parsed.split_sources else None
        group,_=parsed.owner_for(source,component)
        assignments[obj['name']]=group['id']
        checked.append(dict(source_node=source,projection_component=component,hide_render=False))
    # building-048 was merged into the original trunk mesh; preserve its canonical
    # gameplay ownership while validating every actual visible source component.
    actual_sources={r['source_node'] for r in checked}
    assert parsed.sources-actual_sources=={'building-048'}
    checked.append(dict(source_node='building-048',projection_component=None,hide_render=True))
    parsed.validate_meshes(checked)
    old_objects=defaultdict(set);new_objects=defaultdict(set)
    for obj in records:
        old_objects[obj['asset']].add(obj['name']);new_objects[assignments[obj['name']]].add(obj['name'])
    audit=[]
    old_by_id={g['id']:g for g in original['groups']}
    for group in groups:
        identity=group['id'];names=new_objects[identity]
        changed=(old_objects.get(identity)!=names or old_by_id.get(identity,{}).get('name')!=group['name'])
        audit.append(dict(id=identity,name=group['name'],changed=changed,reason=reasons[identity],
            sources=[source_for_part(p) for p in group['parts']],objects=sorted(names),
            previous_assets=sorted({o['asset'] for o in records if o['name'] in names})))
    (output/'catalog.json').write_text(json.dumps(catalog,indent=2)+'\n')
    plan=dict(version=1,source_worker_sha256=sha(EDITOR/'work/needs-review/sherwood-refinement/textures/reproject-v2/source-only.blend'),
              geometry_audit_sha256=sha(geometry),catalog_sha256=sha(output/'catalog.json'),
              original_groups=len(original['groups']),candidate_groups=len(groups),
              assignments=assignments,groups=audit,checks=['every visible mesh assigned exactly once','all original source ownership retained',
              'explicit disjoint selectors for mixed tree/ladder and house/platform sources'])
    (output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print(json.dumps({'before':plan['original_groups'],'after':len(groups),'changed':sum(g['changed'] for g in audit),'meshes':len(assignments)}))


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--geometry',required=True);p.add_argument('--output',required=True)
    a=p.parse_args();build(a.geometry,a.output)

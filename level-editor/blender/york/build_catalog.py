"""Expand York's visually reviewed ownership into the shared catalog schema."""
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/active/york'
CATALOG = ROOT / 'level-editor/refinement/catalogs/york.json'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
    from catalog_schema import parse_catalog
    survey = json.loads((OUT / 'survey/index.json').read_text())
    roots = {row['id']: row['obstacles'] for row in survey}
    level = json.loads((OUT / 'baseline/york.rhp.json').read_text())
    inventory = json.loads((OUT / 'inventory/inventory.json').read_text())
    groups = []
    owners = {}
    for line in Path(__file__).with_name('ownership.txt').read_text().splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        name, tokens = [s.strip() for s in line.split('|')]
        identifier = 'york-' + re.sub('[^a-z0-9]+', '-', name.lower()).strip('-')
        parts = []
        for token in tokens.split():
            component = None
            if token.startswith('c'):
                number, component = token[1:].split(':')
                numbers = [int(number)]
            elif token.startswith('p'):
                numbers = [int(token[1:])]
            else:
                key = f'york-terrace-{int(token[1:]):03}' if token.startswith('t') else f'york-group-{int(token):03}'
                numbers = roots[key]
            for number in numbers:
                obstacle = level['sight_obstacles'][number]
                points = obstacle['points']
                z0 = min(p['z_bottom'] for p in points)
                top_range = max(p['z_top'] for p in points) - min(p['z_top'] for p in points)
                role = 'Raised sloped section' if z0 > 2 and top_range > 3 else 'Raised section' if z0 > 2 else 'Structural volume'
                part = {'obstacle': number, 'name': f'{role} {number:03}'}
                if component:
                    part.update(name='Shared surface / ' + component, components=[component])
                existing = next((p for p in parts if p['obstacle']==number), None)
                if component and existing and 'components' in existing:
                    existing['components'].append(component)
                    existing['name'] = 'Shared surfaces / ' + ', '.join(existing['components'])
                else:
                    parts.append(part)
                owners.setdefault(f'building-{number:03}', identifier)
        groups.append({'id': identifier, 'name': name, 'parts': sorted(parts, key=lambda p:p['obstacle'])})
    expected = {r['source_node'] for r in inventory['objects']} - {'ground'}
    catalog = {'version': 2, 'map': 'york', 'groups': groups, 'canonical_owners': owners,
               'terrain': {'source_node': 'ground', 'id': 'york-terrain', 'name': 'York river and background terrain',
                           'role': 'Painted ground and river; raised terrain has separate named owners'},
               'grouping_policy': 'One asset per distinct building, including adjoining houses. Towers and gatehouses are independent assets even when attached. Roofs, walls and details stay with their individual building.',
               'review_scope': 'Complete imported static geometry ownership; state-only sprites and absent geometry are separately inventoried.'}
    index = parse_catalog(catalog, expected)
    missing = sorted(set(range(len(level['sight_obstacles']))) - {int(s.split('-')[1]) for s in expected})
    associations = {115:111,119:116,190:184,319:314,329:328,356:344,859:858,934:935,938:939}
    catalog['nonrendering_sources'] = [{'obstacle': n, 'owner': owners[f'building-{associations[n]:03}'],
        'reason': 'No mesh in frozen reconstruction; native gameplay record retained in frozen level data. Do not fabricate visible geometry during grouping.'} for n in missing]
    catalog['partitions'] = [{'source_node': 'building-650', 'coordinate': 'game_y - 0.325 * game_x',
        'boundaries': [669.0,716.0,774.0,832.0],
        'components_ascending': ['north-house','red-roof-house','dormer-house','gable-house','front-shop'],
        'rationale': 'Historical frontage slices now belong together to the shared occlusion-volume asset. Retain their complete surface union and UVs; this non-solid source is not an individual building foundation.'}]
    for number in [769,795]:
        catalog['partitions'].append({'source_node': f'building-{number:03}',
            'coordinate': 'game_y + 0.305 * game_x', 'axis_coefficients': [0.305,1.0],
            'boundaries': [1838.5], 'components_ascending': ['hall','tower'],
            'rationale': 'Shared lower volume and east wall span the hall and tower. Split along the hall end-wall direction through its native junction; hidden continuation is inferred. Preserve all surfaces and UVs without adding caps.'})
    catalog['partitions'].extend(json.loads(Path(__file__).with_name('review-partitions.json').read_text()))
    CATALOG.write_text(json.dumps(catalog, indent=2)+'\n')
    layers = json.loads((OUT / 'source-states-complete/layers.json').read_text())
    states = []
    for patch in layers['patches'] + layers['mission_patches']:
        nodes = sorted(set(patch['sight_before'] + patch['sight_after']))
        names = sorted({owners[n] for n in nodes if n in owners})
        states.append({'id':patch['id'],'name':patch['name'],'owners':names,
                       'sight_before':patch['sight_before'],'sight_after':patch['sight_after'],
                       'scope':'Native activation association; does not imply validated texture or interior receiver ownership'})
    supplement = {'version':1,'map':'york','states':states,'sprite_only_assets':[
        {'id':'york-castle-portcullis','name':'Castle courtyard portcullis','patch':'patch-000','owner':'york-castle-west-gatehouse','status':'Source sprite inventoried; independent mesh absent'},
        {'id':'york-castle-gate-mechanism','name':'Castle gate mechanism','patch':'patch-004','status':'State record inventoried; standalone geometry requires the next refinement phase'},
        {'id':'york-south-wall-lane-door','name':'South wall-lane mission door','patch':'mission-Str03_Yor_MK-patch-000','owner':None,'status':'Fog animation frames frozen; independent mesh absent; architectural receiver requires review'},
        {'id':'york-inner-east-wall-door','name':'Inner east wall mission door','patch':'mission-Str03_Yor_MK-patch-001','owner':None,'status':'Fog animation frames frozen; independent mesh absent; architectural receiver requires review'},
    ],'unmodeled_scenery':{'scope':'Painted vegetation, snow, ground clutter and decorative facade detail remain on the background or structural textures. This grouping pass does not create new scenery geometry.'}}
    (OUT/'state-ownership.json').write_text(json.dumps(supplement,indent=2)+'\n')
    result = {'status':'PASS','groups':len(groups),'visible_source_parts':len(index.sources),
              'nonrendering_source_records':len(missing),'total_native_obstacles':len(level['sight_obstacles']),
              'shared_sources':sorted(index.split_sources),'duplicate_component_owners':0,
              'catalog_sha256':sha(CATALOG),'inventory_sha256':sha(OUT/'inventory/inventory.json')}
    (OUT/'catalog-validation.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()

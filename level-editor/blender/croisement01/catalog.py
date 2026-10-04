"""Seed explicit source ownership; unfinished groups remain visibly provisional."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement01-refinement'
PROPS = [
    ('east-ivy-stump', 'East Ivy Stump', [54], 67),
    ('southwest-cut-stump', 'Southwest Cut Stump', [55], 65),
    ('south-cut-stump', 'South Cut Stump', [56, 67], 66),
    ('central-ivy-stump', 'Central Ivy Stump', [57, 58], 69),
    ('southeast-small-stump', 'Southeast Small Stump', [59], 68),
    ('southwest-broken-stump', 'Southwest Broken Stump', [60], 64),
    ('east-fallen-branch', 'East Fallen Branch', [68], 70),
]


def main():
    groups = [dict(id='croisement01-'+slug, name=name,
                   parts=[dict(obstacle=i, name=f'{name} part {i:03}') for i in ids],
                   source_mask=mask, status='geometry refinement in progress')
              for slug, name, ids, mask in PROPS]
    assigned = {p['obstacle'] for g in groups for p in g['parts']}
    for index in range(85):
        if index in assigned:
            continue
        kind = 'Applied State' if index in (83, 84) else 'Terrain' if index in range(10) or index in range(76,81) else 'Unresolved Source Part'
        groups.append(dict(id=f'croisement01-source-{index:03}', name=f'{kind} {index:03}',
                           parts=[dict(obstacle=index, name=f'Source part {index:03}')],
                           status='ownership refinement in progress'))
    ids = [p['obstacle'] for g in groups for p in g['parts']]
    assert sorted(ids) == list(range(85))
    (OUT/'catalog.json').write_text(json.dumps(dict(version=1, map='Croisement01', groups=groups,
        review_notes='All native parts accounted for; only named prop assemblies have visually reviewed grouping. Provisional parts, mask-only vegetation, crowns, terrain and mission visuals remain unfinished.'), indent=2)+'\n')
    print(len(groups), 'groups; 85 native parts; 7 named prop assemblies')


if __name__ == '__main__':
    main()

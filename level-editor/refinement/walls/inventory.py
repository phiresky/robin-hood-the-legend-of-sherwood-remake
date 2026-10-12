"""Inventory wall candidates without trusting asset names or changing shared assets."""
import hashlib
import json
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[2]
LIBRARY = ROOT / 'library/3d-assets'
OUTPUT = ROOT / 'work/needs-review/wall-presets'

def inventory():
    entries = json.loads((LIBRARY/'index.json').read_text())['assets']
    result = []
    for entry in entries:
        if entry['source_map'].lower() == 'wychford' or entry['id'].startswith('spline-'):
            continue
        raw = (LIBRARY/entry['descriptor']).read_bytes()
        descriptor = json.loads(raw)
        points = [p for part in descriptor.get('parts', []) for p in (part.get('obstacle_local_game') or {}).get('points', [])]
        bounds = descriptor.get('bounds_local_scene')
        angle, dimensions = None, None
        if len(points) >= 3:
            xy = np.array([[p['x'], -p['y']/np.sin(np.deg2rad(35))] for p in points])
            centered = xy - xy.mean(axis=0)
            _, axes = np.linalg.eigh(centered.T @ centered)
            direction = axes[:, -1]
            angle = float(np.rad2deg(np.arctan2(direction[1], direction[0])))
            projected = centered @ axes
            dimensions = [float(np.ptp(projected[:, 1])), float(np.ptp(projected[:, 0])),
                          float((max(p['z_top'] for p in points)-min(p['z_bottom'] for p in points))/np.cos(np.deg2rad(35)))]
        elif bounds:
            dimensions = [b-a for a,b in zip(bounds['min'], bounds['max'])]
            dimensions[:2] = sorted(dimensions[:2], reverse=True)
        text = (entry['id']+' '+entry['name']).lower()
        named = any(w in text for w in ['wall','fence','curtain','turret','tower','palisad'])
        elongated = dimensions and dimensions[0] > 70 and dimensions[0]/max(dimensions[1],1) > 2 and 8 < dimensions[2] < dimensions[0]*1.7
        result.append(dict(id=entry['id'], name=entry['name'], source_map=entry['source_map'],
                           candidate=bool(named or elongated), named=named, angle=angle, dimensions=dimensions,
                           source_descriptor_sha256=hashlib.sha256(raw).hexdigest(),
                           descriptor=entry['descriptor'], model=entry['model'],
                           source_nodes=[p['node'] for p in descriptor.get('parts', [])]))
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT/'inventory.json').write_text(json.dumps(result,indent=2)+'\n')
    candidates=[x for x in result if x['candidate']]
    (OUTPUT/'candidates.json').write_text(json.dumps(candidates,indent=2)+'\n')
    from collections import Counter
    print(len(result),'assets audited;',len(candidates),'candidates;',dict(Counter(x['source_map'] for x in candidates)))

if __name__ == '__main__': inventory()

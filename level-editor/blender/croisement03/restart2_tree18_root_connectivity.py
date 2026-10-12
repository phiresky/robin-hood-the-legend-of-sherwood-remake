"""Correct a reversed scaffold ring without moving source-supported wood sections."""
import copy
import hashlib
import json
from collections import Counter
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/croisement03/restart2'


def crossing_pairs(part):
    """Strict interior segment/triangle crossings; coplanar contacts are excluded."""
    v = np.array(part['vertices'])
    f = part['faces']
    crossed = []
    for i, aa in enumerate(f):
        for j in range(i + 1, len(f)):
            bb = f[j]
            if set(aa) & set(bb):
                continue
            found = False
            for edgeface, triangle in [(aa, bb), (bb, aa)]:
                p, q, r = v[triangle]
                e1, e2 = q - p, r - p
                for a, b in zip(edgeface, edgeface[1:] + edgeface[:1]):
                    direction = v[b] - v[a]
                    h = np.cross(direction, e2)
                    det = np.dot(e1, h)
                    if abs(det) < 1e-10:
                        continue
                    s = v[a] - p
                    u = np.dot(s, h) / det
                    qcross = np.cross(s, e1)
                    w = np.dot(direction, qcross) / det
                    t = np.dot(e2, qcross) / det
                    if 1e-8 < t < 1 - 1e-8 and u > 1e-8 and w > 1e-8 and u + w < 1 - 1e-8:
                        found = True
            if found:
                crossed.append([i, j])
    return crossed


def main():
    source = BASE / 'tree18-cpu-wood-v2/geometry.json'
    out = BASE / 'tree18-root-connectivity-cpu-v1'
    assert not out.exists()
    original = json.loads(source.read_text())
    revised = copy.deepcopy(original)
    part = next(p for p in revised['parts'] if p['track'] == 'main_lower')
    oldpart = next(p for p in original['parts'] if p['track'] == 'main_lower')
    n = part['ring_sides']
    assert part['ring_roles'][0] == 'inferred_ground_scaffold'
    ring = part['vertices'][:n]
    part['vertices'][:n] = [ring[(-i) % n] for i in range(n)]
    assert sorted(part['vertices'][:n]) == sorted(ring)
    assert part['vertices'][n:] == oldpart['vertices'][n:]
    vv = np.array(part['vertices'])
    volume = sum(np.dot(vv[a], np.cross(vv[b], vv[c])) / 6 for a, b, c in part['faces'])
    assert volume > 0
    part['volume'] = float(volume)
    part['minimum_triangle_area'] = float(min(np.linalg.norm(np.cross(vv[b] - vv[a], vv[c] - vv[a])) / 2 for a, b, c in part['faces']))
    edges = Counter(tuple(sorted((a, b))) for f in part['faces'] for a, b in zip(f, f[1:] + f[:1]))
    assert set(edges.values()) == {2}
    assert len(vv) - len(edges) + len(part['faces']) == 2
    before = crossing_pairs(oldpart)
    after = crossing_pairs(part)
    assert before and not after, (before, after)
    assert all(part['vertices'][g['vertex']] == oldpart['vertices'][g['vertex']] for g in part['source_endpoint_guards'])
    for p, old in zip(revised['parts'], original['parts']):
        if p['track'] != 'main_lower':
            assert p == old
    proof = {
        'status': 'PASS_SCOPED_RING_CONNECTIVITY_REQUIRES_SAVED_REVIEW',
        'input': str(source), 'input_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
        'old_root_ring': ring, 'permutation': [(-i) % n for i in range(n)],
        'strict_interior_crossing_pairs_before': before, 'strict_interior_crossing_pairs_after': after,
        'all_source_sections_exact': True, 'root_world_point_set_exact': True,
        'other_three_parts_exact': True, 'closed_euler': 2, 'volume': float(volume),
        'limits': ['Root location remains inferred and requires bank/terrain contact review.',
                   'Strict test excludes coplanar contacts and triangle pairs sharing vertices; saved Boolean and eight-view checks remain required.',
                   'No material, source ownership, native priority or gameplay changes.']}
    revised['root_connectivity_correction'] = proof
    revised['status'] = 'HOLD_PRIVATE_ROOT_CONNECTIVITY_CORRECTED_NOT_UNIONED'
    out.mkdir()
    (out / 'geometry.json').write_text(json.dumps(revised, indent=2) + '\n')
    (out / 'receipt.json').write_text(json.dumps(proof, indent=2) + '\n')
    print({'before_crossings': len(before), 'after_crossings': len(after), 'output': str(out)})


if __name__ == '__main__':
    main()

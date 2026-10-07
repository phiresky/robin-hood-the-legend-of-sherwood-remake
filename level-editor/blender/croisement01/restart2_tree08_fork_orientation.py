"""Orient an isolated closed fork by edge adjacency without moving geometry."""
import json
from collections import defaultdict
from pathlib import Path

import numpy as np

from restart2_tree08_local_fork import audit


def orient(faces):
    edges = defaultdict(list)
    for i, face in enumerate(faces):
        for a, b in zip(face, np.roll(face, -1)):
            edges[tuple(sorted((int(a), int(b))))].append((i, int(a) < int(b)))
    assert all(len(entries) == 2 for entries in edges.values())
    adjacency = [[] for _ in faces]
    for entries in edges.values():
        (a, da), (b, db) = entries
        adjacency[a].append((b, da == db))
        adjacency[b].append((a, da == db))
    flips = {}
    components = 0
    for start in range(len(faces)):
        if start in flips:
            continue
        components += 1
        flips[start] = False
        stack = [start]
        while stack:
            current = stack.pop()
            for other, opposite in adjacency[current]:
                value = flips[current] ^ opposite
                if other in flips:
                    assert flips[other] == value, 'Nonorientable component'
                else:
                    flips[other] = value
                    stack.append(other)
    result = faces.copy()
    indices = [i for i, value in flips.items() if value]
    result[indices] = result[indices, ::-1]
    return result, indices, components


def main():
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    packet = root / 'tree08-v12-local-fork-kernel-v1'
    data = np.load(packet / 'pair-29.npz')
    vertices, faces = data['vertices'], data['faces']
    corrected, flipped, components = orient(faces)
    if audit(vertices, corrected)['volume'] < 0:
        corrected = corrected[:, ::-1]
        flipped = sorted(set(range(len(faces))) - set(flipped))
    check = audit(vertices, corrected)
    assert components == 1
    assert not any(check[k] for k in ['nonmanifold_edges', 'winding_errors', 'zero_area'])
    assert np.array_equal(np.sort(faces, axis=1), np.sort(corrected, axis=1))
    output = root / 'tree08-v12-local-fork-oriented-v1'
    output.mkdir(exist_ok=False)
    np.savez_compressed(output / 'pair-29.npz', vertices=vertices, faces=corrected)
    report = dict(status='Closed orientation PASS; geometric intersection/source proof pending',
                  source_packet=str(packet.relative_to(root)), flipped_face_indices=flipped,
                  connected_components=components, topology=check,
                  vertex_displacement=0., geometric_triangle_changes=0,
                  prior_signed_volume=audit(vertices, faces)['volume'],
                  signed_volume_note='Signed sum changes when inward faces are corrected. The unoriented geometric surface and enclosed region are exactly unchanged.',
                  limitation='No final saved model or rendering. VTK precision displacement, self-intersections and native source visibility are not yet certified.')
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()

"""Resume original-surface clipping only across declared rooted attachments."""
import argparse
import hashlib
import json
import subprocess
import shutil
from pathlib import Path

import numpy as np

from restart2_tree08_cap_plan import read_paths, section_loops, serialize, triangulate


def inputs(root):
    source = root / 'tree08-v12-chain-cpu-v3'
    report = json.loads((source / 'report.json').read_text())
    assert hashlib.sha256((source / 'mesh.npz').read_bytes()).hexdigest() == report['mesh_sha256']
    plan = json.loads((source / 'fork-union-plan.json').read_text())
    archive = np.load(source / 'mesh.npz')
    origin = np.array([552., -672., 235.])
    cluster_path = root / 'tree08-v12-fiveway-stitched-v3/candidate.npz'
    assert hashlib.sha256(cluster_path.read_bytes()).hexdigest() == '9a02dda032abd49f2f0cf0e253024ddf057beb4b47f943773d7dd955c8fdff8e'
    replaced = {29, 30, 31, 33, 93, 96}
    cluster = np.load(cluster_path)
    meshes = {i: (archive[f'vertices_{i}'] - origin, archive[f'faces_{i}'])
              for group in plan['groups'] for i in group if i not in replaced}
    meshes[200] = (cluster['vertices'] - origin, cluster['faces'])
    mapping = lambda i: 200 if i in replaced else i
    adjacency = {i: set() for i in meshes}
    for edge in plan['rooted_attachment_hypotheses']:
        a, b = mapping(edge['a']), mapping(edge['b'])
        if a != b:
            adjacency[a].add(b)
            adjacency[b].add(a)
    groups = [sorted({mapping(i) for i in group}) for group in plan['groups']]
    return meshes, adjacency, groups, origin


def reconstruct(index, meshes, adjacency, origin, kernel, destination):
    vertices, faces = meshes[index]
    neighbors = sorted(adjacency[index])
    bounds = {j: (meshes[j][0].min(0), meshes[j][0].max(0)) for j in neighbors}
    surfaces = []
    untouched, clipped = 0, 0
    for face_index, face in enumerate(faces):
        triangle = vertices[face]
        low, high = triangle.min(0), triangle.max(0)
        local_neighbors = [j for j, (lo, hi) in bounds.items() if not (np.any(high < lo) or np.any(low > hi))]
        if not local_neighbors:
            surfaces.append(triangle)
            untouched += 1
            continue
        center = triangle.mean(0)
        normal = np.cross(triangle[1] - triangle[0], triangle[2] - triangle[0])
        normal /= np.linalg.norm(normal)
        u = triangle[0] - center
        u /= np.linalg.norm(u)
        basis = np.array([u, np.cross(normal, u)])
        loops, patches = [], []
        try:
            for j in local_neighbors:
                loops.extend(section_loops(*meshes[j], center, normal, basis))
            if not loops:
                surfaces.append(triangle)
                untouched += 1
                continue
            subject = (triangle - center) @ basis.T
            result = subprocess.run([str(kernel)], input=serialize([subject]) + serialize(loops),
                                    text=True, check=True, capture_output=True, timeout=10)
            patches = read_paths(iter(result.stdout.split()))
            output = triangulate(patches) if patches else []
        except Exception as error:
            failure = dict(status='HOLD exact local input', section=index, face=face_index,
                           neighbors=local_neighbors, error=repr(error),
                           subject=((triangle-center) @ basis.T).tolist(),
                           clips=[p.tolist() for p in loops], patches=[p.tolist() for p in patches],
                           origin=(origin+center).tolist(), basis=basis.tolist())
            failure_path = destination / f'failure-{index}.json'
            if failure_path.exists():
                failure_path = destination / f'failure-{index}-face{face_index}.json'
            with failure_path.open('x') as stream:
                json.dump(failure, stream, indent=2)
                stream.write('\n')
            raise
        for patch in output:
            world = center + patch @ basis
            if np.dot(np.cross(world[1]-world[0], world[2]-world[0]), normal) < 0:
                world = world[::-1]
            surfaces.append(world)
        clipped += 1
    # Unindexed per-section triangles are intentionally provisional. Joining and
    # edge conformity happen once all declared neighboring surfaces are present.
    triangles = np.asarray(surfaces).reshape(-1, 3, 3) + origin
    np.savez_compressed(destination / f'part-{index}.npz', triangles=triangles)
    result = dict(section=index, neighbors=neighbors, untouched_triangles=untouched,
                  reconstructed_triangles=clipped, output_triangles=len(triangles))
    (destination / f'part-{index}.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--group', type=int, choices=[0, 1], required=True)
    parser.add_argument('--section', type=int)
    parser.add_argument('--retry-failed', action='store_true')
    parser.add_argument('--kernel-version', type=int, choices=[1, 2], default=1)
    parser.add_argument('--local-contacts', action='store_true')
    parser.add_argument('--revision', type=int, default=1)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    output = root / f'tree08-v12-remaining-group{args.group}-v{args.revision}'
    output.mkdir(exist_ok=True)
    meshes, adjacency, groups, origin = inputs(root)
    contact_plan = None
    if args.local_contacts:
        assert args.group == 0 and args.revision > 1
        contact_plan = json.loads((root / 'tree08-v12-local-contact-plan.json').read_text())
        for a, b in contact_plan['additional_contact_pairs']:
            adjacency[a].add(b)
            adjacency[b].add(a)
        for index in groups[args.group]:
            if index not in contact_plan['affected_sections']:
                for extension in ['npz', 'json']:
                    target = output / f'part-{index}.{extension}'
                    if not target.exists():
                        shutil.copy2(root / f'tree08-v12-remaining-group{args.group}-v1/part-{index}.{extension}', target)
    (output / 'scope.json').write_text(json.dumps(dict(group=args.group, sections=groups[args.group],
        local_contact_plan=contact_plan,
        basis='Rooted attachment edges and explicit overlapping-junction neighborhoods only; held crossings excluded; no blanket spatial union'), indent=2) + '\n')
    ids = [args.section] if args.section is not None else groups[args.group]
    failures = []
    for index in ids:
        assert index in groups[args.group]
        if (output / f'part-{index}.npz').exists():
            continue
        if (output / f'failure-{index}.json').exists() and not args.retry_failed:
            failures.append(index)
            continue
        print('BEGIN SECTION', index, 'neighbors', sorted(adjacency[index]), flush=True)
        try:
            result = reconstruct(index, meshes, adjacency, origin,
                                 root / ('tree08-v12-local-fork-cpu-v1/cap-clip' if args.kernel_version == 1 else 'tree08-v12-local-fork-cpu-v1/cap-clip-v2'), output)
            print('COMPLETE', result, flush=True)
        except Exception as error:
            print('HOLD SECTION', index, repr(error), flush=True)
            failures.append(index)
    print('GROUP CPU PIECES COMPLETE; failed sections', failures, flush=True)


if __name__ == '__main__':
    main()

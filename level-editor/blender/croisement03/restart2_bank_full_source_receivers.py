"""Read-only first-hit inventory for proposed full-bank source domains, not ownership approval."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as guard


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--proposal', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    assert not args.output.exists()
    proof_path = args.proposal/'receipt.json'
    proof = json.loads(proof_path.read_text())
    for name, digest in proof['pins'].items():
        assert guard.sha(Path(name)) == digest, name
    native = np.array(Image.open(args.proposal/'native-foreground.png')) > 0
    uncertain = np.array(Image.open(args.proposal/'manual-ambiguous.png')) > 0
    domains = {}
    for name in ('additional-interior-proposal', 'boundary-review'):
        domains[name] = np.array(Image.open(args.proposal/(name+'.png'))) > 0
        assert not np.any(domains[name] & (native | uncertain))
    assert int(domains['additional-interior-proposal'].sum()) == proof['additional_interior_proposals']
    pins = {str(p.resolve()): guard.sha(p) for p in [args.model, proof_path, *args.proposal.glob('*.png')]}
    guard.resources()
    guard.acquire(slots=2)
    try:
        guard.resources()
        bpy.ops.wm.open_mainfile(filepath=str(args.model))
        rows = []
        for obj in bpy.context.scene.objects:
            if obj.type != 'MESH':
                continue
            assert not any(m.show_render for m in obj.modifiers), ('Unhandled evaluated modifier', obj.name)
            mesh = obj.data
            mesh.calc_loop_triangles()
            tri = list(mesh.loop_triangles)
            vertices = [obj.matrix_world @ v.co for v in mesh.vertices]
            rows.append((obj.name, BVHTree.FromPolygons(vertices, [list(t.vertices) for t in tri], all_triangles=True), tri))
        ray = Vector((0, -guard.C, guard.S))
        results = {}
        for category, domain in domains.items():
            hits = []
            owners = {}
            for y, x in np.argwhere(domain):
                origin = Vector((float(x)+.5, -(float(y)+.5)/guard.S, 0)) + ray*5000
                intersections = []
                for name, tree, triangles in rows:
                    p, n, f, d = tree.ray_cast(origin, -ray)
                    if p is not None:
                        intersections.append((d, name, triangles[f].polygon_index, list(p)))
                first = min(intersections) if intersections else None
                owner = first[1] if first else 'NO_RECEIVER'
                owners[owner] = owners.get(owner, 0)+1
                hits.append({'source_xy': [int(x), int(y)], 'firsthit': first, 'status': 'candidate_bank_receiver' if owner.startswith('Candidate bank ') else 'unresolved_receiver'})
            results[category] = {'pixels': len(hits), 'receiver_counts': owners, 'rays': hits}
        for p, digest in pins.items():
            assert guard.sha(Path(p)) == digest, p
        report = {'status': 'PRIVATE_RECEIVER_INVENTORY_NOT_OWNERSHIP', 'pins': pins, 'tested_objects': [r[0] for r in rows], 'domains': results, 'limits': ['Source patch classification does not establish a particular52/53/54 mesh owner.', 'All saved meshes are tested geometrically; this bank worker has no complete tree/ivy scene, so neighbor/native alpha priority requires separate full context checks.', 'No materials, geometry, source pixels or model bytes changed. Boundary samples are diagnostic only and remain unassigned.']}
        payload = json.dumps(report, separators=(',', ':'))+'\n'
        assert len(payload.encode()) < 32*1024**2
        args.output.mkdir()
        (args.output/'receipt.json').write_text(payload)
        print({k: {'pixels': v['pixels'], 'receivers': v['receiver_counts']} for k, v in results.items()})
    finally:
        guard.release()


if __name__ == '__main__':
    main()

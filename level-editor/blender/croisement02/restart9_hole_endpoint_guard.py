"""Audit saved endpoint geometry against its private terrain aperture contract."""
import sys, json, math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from restart9_hole_contact import ROOT, PINS, point, RAY, SIN, COS
from evidence_io import sha, write_json
from render_slots import acquire, release


def main():
    dest = ROOT/'endpoint-guard-v1'
    dest.mkdir(parents=True, exist_ok=False)
    audit = json.loads((ROOT/'receiver-audit-v2/report.json').read_text())
    plan = json.loads((ROOT/'source-plan-v1/plan.json').read_text())
    results = []
    for phase, (folder, digest) in PINS.items():
        model = ROOT/folder/'model.blend'
        assert sha(model) == digest
        bpy.ops.wm.open_mainfile(filepath=str(model))
        bpy.context.view_layer.update()
        objects = [o for o in bpy.context.scene.objects if o.type == 'MESH']
        assert len(objects) == 1
        ob = objects[0]
        mesh = ob.data
        mesh.calc_loop_triangles()
        triangles = list(mesh.loop_triangles)
        points = [ob.matrix_world@v.co for v in mesh.vertices]
        tree = BVHTree.FromPolygons(points, [tuple(t.vertices) for t in triangles], all_triangles=True)
        frame = next(r for r in plan['phases'] if r['phase'] == phase)
        rgba = np.array(Image.open(frame['source']).convert('RGBA'))
        failures, depth = [], []
        for y, x in np.argwhere(rgba[:,:,3] > 0):
            sx, sy = float(x)+.5+frame['offset'][0], float(y)+.5+frame['offset'][1]
            p, n, index, distance = tree.ray_cast(point(sx,sy,0)+RAY*3000, -RAY)
            assert p is not None
            r2 = ((sx-49)/17)**2+((sy-56)/12)**2
            # The local terrain remains intact outside the applied aperture.
            blocked = p.z < -1e-4 and (phase == 'initial' or r2 >= 1.)
            if blocked:
                failures.append([int(x),int(y),float(p.z),r2])
            depth.append(float(p.z))
        assert not failures, failures
        below_outside = []
        for i, p in enumerate(points):
            if p.z >= -.46:
                continue  # Thin leaf undersides may meet/penetrate the substrate.
            r2 = ((p.x-49)/17)**2+((-p.y*SIN-56)/12)**2
            if r2 > 1.002:
                below_outside.append([i,list(p),r2])
        assert not below_outside, below_outside[:10]
        placements=[]
        for row in audit['positions']:
            z = row['center']['hit'][2]
            if abs(z) < .002:
                z = 0.
            anchor = row['display_position']
            translation = point(anchor[0],anchor[1],z)
            drift=max(max(abs((p+translation).x-p.x-anchor[0]),
                          abs((-(p+translation).y*SIN-(p+translation).z*COS)-(-p.y*SIN-p.z*COS)-anchor[1]))
                      for p in points)
            assert drift < .002, drift
            placements.append(dict(display_position=anchor, translation=list(translation), support_z=z,
                                   instances=row['instances'], maximum_projected_translation_error=drift))
        results.append(dict(phase=phase, model=str(model), model_sha256=digest,
                            source_centers=int((rgba[:,:,3]>0).sum()), terrain_occluded_centers=failures,
                            source_hit_z_range=[min(depth),max(depth)], below_ground_outside_aperture=below_outside,
                            placements=placements, placement_instance_count=sum(len(r['instances']) for r in placements)))
    write_json(dest/'report.json', dict(status='PASS saved physical endpoints and bounded aperture contract',
                receiver_audit_sha256=sha(ROOT/'receiver-audit-v2/report.json'), endpoints=results,
                method='Saved mesh native center rays tested against the explicit flat receiver/aperture equation; each saved vertex translated to all17 sampled support planes with original projection fixed. Native elevation remains unchanged.',
                limits=['Terrain support audit is finite sampling, not arbitrary foreground occlusion proof.',
                        'Thin leaf undersides may penetrate substrate by up to0.45 units; no deep bowl volume exits the aperture.',
                        'Runtime state-local receiver replacement remains unimplemented; this is a private physical contract.']))


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()

"""Sample private leaf packets against measured, immutable static receiver points."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from scipy.spatial import cKDTree

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart2_check_sign_bend_ground import samples


def main():
    root=OUT/'restart8-hidden-archer-leaf-trial-v1'
    authority=OUT/'restart8-hidden-archer-receiver-audit-v1/current-first-hit-v1/report.json'
    hits=json.loads(authority.read_text())
    for folder in sorted(root.glob('profile-*')):
        output=folder/'support-samples-v1.json'
        assert not output.exists()
        record=json.loads((folder/'construction.json').read_text())
        model=folder/'model.blend';assert sha(model)==record['model_sha256']
        number=folder.name.split('-')[1]
        row=next(p for p in hits['profiles'] if p['profile'].endswith(number))
        wood=[s for s in row['samples'] if s['world'] and 'tree-' in s['asset']]
        ground=[s for s in row['samples'] if s['world'] and s['asset']=='croisement02-ground-receiver']
        trees=[cKDTree(np.array([s['world'] for s in values])) for values in [wood,ground]]
        bpy.ops.wm.open_mainfile(filepath=str(model))
        obj=next(o for o in bpy.context.scene.objects if o.type=='MESH')
        points,records=samples(obj,with_records=True)
        distances=[tree.query(points)[0] for tree in trees]
        packet_points={}
        for point,entry in zip(points,records):
            face=obj.data.polygons[entry[0]]
            packet=int(min(face.vertices)//16)
            packet_points.setdefault(packet,[]).append(point)
        packets=[]
        for packet,values in packet_points.items():
            values=np.array(values)
            packets.append(dict(packet=packet,opaque_samples=len(values),
                minimum_z=float(values[:,2].min()),maximum_z=float(values[:,2].max()),
                measured_wood_point_distance_upper_bound=float(trees[0].query(values)[0].min()),
                measured_ground_point_distance_upper_bound=float(trees[1].query(values)[0].min())))
        write_json(output,dict(status='Finite support diagnostic; not a closed-volume contact certificate',
            model_sha256=sha(model),static_point_authority_sha256=sha(authority),
            method='Opaque UV samples every two texels; Euclidean distance to independently measured static first-hit points',
            opaque_samples=len(points),minimum_opaque_z=float(points[:,2].min()),
            wood_samples_within_two_units=int((distances[0]<=2).sum()),
            ground_samples_within_two_units=int((distances[1]<=2).sum()),
            packets=packets,limits=[
                'A nearby measured point proves a finite distance upper bound; a distant point does not prove no contact elsewhere.',
                'Opaque leaf cards are disconnected surfaces; packet support and inferred branch structure require visual review.',
                'Static receiver interiors and intersections are not classified by this point-cloud test.',
                'All source receivers and candidate models remain unchanged.']))
        assert sha(model)==record['model_sha256']


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()

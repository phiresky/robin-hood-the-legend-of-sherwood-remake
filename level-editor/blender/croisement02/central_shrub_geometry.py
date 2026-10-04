"""Native-only clumps with explicit bank contact and a tapered boundary conifer."""
import json
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from scipy.ndimage import gaussian_filter1d
from catalog import OUT,scenery_workspace
from evidence_io import sha,write_json
from shrub_geometry import build as build_clump
from tree_geometry import SIN,COS,RAY
from opacity_bounds import measure
SUPPORT=None


def load_support():
    global SUPPORT
    worker=scenery_workspace('croisement02-north-woodland-bank');digest=sha(worker/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    objects=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')==worker.name]
    if sorted(o.get('source_node') for o in objects)!=[f'building-{i:03}' for i in range(5)]:raise ValueError('Unexpected bank support scope')
    SUPPORT=(worker,digest,[(o.name,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices) for p in o.data.polygons])) for o in objects])


def build(obj,packet):
    if SUPPORT is None:raise ValueError('Load audited bank before source scene')
    if packet['native_mask']==73:
        from conifer_geometry import build as build_conifer
        result=build_conifer(obj,packet)
    else:
        result=build_clump(obj,packet)
    points=[v.co.copy() for v in obj.data.vertices]
    lo=Vector([min(p[a] for p in points) for a in range(3)]);hi=Vector([max(p[a] for p in points) for a in range(3)]);center=(lo+hi)/2
    bank,digest,support=SUPPORT
    def surface(distance):
        x,y=center.x,center.y-distance*COS;hits=[]
        for name,bvh in support:
            p,n,f,d=bvh.ray_cast(Vector((x,y,1000)),Vector((0,0,-1)))
            if p is not None:hits.append((p.z,name))
        if packet['native_mask'] in (68,69,70,71,72,75,79,80,82,91) and not hits:
            return (0.,'ground-plane; central source context outside bank')
        if not hits:raise ValueError(f'Plant{packet["native_mask"]} support outside bank at {x},{y}; inspect before assuming ground')
        return max(hits)
    def residual(distance):return lo.z+distance*SIN-surface(distance)[0]-.5
    low=-32.;high=128.
    while residual(low)>0:low*=2
    while residual(high)<0:high*=2
    for _ in range(50):
        mid=(low+high)/2
        if residual(mid)<0:low=mid
        else:high=mid
    distance=(low+high)/2;terrain,part=surface(distance)
    for v in obj.data.vertices:v.co+=RAY*distance
    obj.data.update()
    if sha(bank/'model.blend')!=digest:raise ValueError('Bank changed during support solve')
    proof=dict(native_mask=packet['native_mask'],bank_worker=str(bank),bank_model_sha256=digest,ray_translation=distance,
               world_translation=[0,-distance*COS,distance*SIN],support_point=[center.x,center.y-distance*COS,terrain],support_part=part,
               minimum_elevation=min(v.co.z for v in obj.data.vertices),clearance=.5,projection_delta=[0,0],
               limitation='Center support on exact bank; low leaf intersections require grouped oblique review.')
    write_json(Path(packet['directory'])/'support.json',proof)
    if packet.get('inferred_branch_support'):
        from central_support_geometry import append_support
        result['inferred_branch_support']=append_support(obj,packet,terrain)
    result.update(minimum_z=proof['minimum_elevation'],opacity_bounds=measure(obj),support=proof,references=[],
                  native_only=True,source_projection_preserved=True)
    return result

"""Place unapproved shrubs65/66 on audited bank support along source-camera rays."""
import json
import sys
from pathlib import Path
import bpy
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from catalog import OUT,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS
from refine_shrubs import main as refine


def main():
    bank=scenery_workspace('croisement02-north-woodland-bank')
    bank_hash=sha(bank/'model.blend')
    decisions={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    bpy.ops.wm.open_mainfile(filepath=str(bank/'model.blend'))
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==bank.name]
    if sorted(o.get('source_node') for o in objects)!=[f'building-{i:03}' for i in range(5)]:raise ValueError('Unexpected bank scope')
    support=[(o.name,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices) for p in o.data.polygons])) for o in objects]
    def surface(x,y):
        hits=[]
        for name,bvh in support:
            point,normal,face,distance=bvh.ray_cast(Vector((x,y,1000)),Vector((0,0,-1)))
            if point is not None:hits.append((point.z,name))
        if not hits:raise ValueError(f'Shifted shrub center is outside audited bank support: {x},{y}')
        return max(hits)
    records=[]
    for index in (65,66):
        worker=OUT/f'understory-round-1/assets/croisement02-shrub-{index:02}'
        if decisions.get(worker.name,{}).get('decision')=='approved':raise ValueError('Approved shrub is frozen')
        with bpy.data.libraries.load(str(worker/'model.blend'),link=False) as (source,loaded):loaded.collections=['Croisement02 Working']
        collection=loaded.collections[0];bpy.context.scene.collection.children.link(collection);bpy.context.view_layer.update()
        obj=next(o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==worker.name)
        if any(abs(obj.matrix_world[r][c]-Matrix.Identity(4)[r][c])>1e-7 for r in range(4) for c in range(4)):raise ValueError('Authored shrub transform must be identity')
        points=[v.co.copy() for v in obj.data.vertices]
        lo=Vector([min(p[a] for p in points) for a in range(3)]);hi=Vector([max(p[a] for p in points) for a in range(3)])
        center=(lo+hi)/2
        def residual(distance):return lo.z+distance*SIN-surface(center.x,center.y-distance*COS)[0]-.5
        if residual(0)>=-.01:raise ValueError('Already supported; do not accumulate placement revisions')
        lower=0.;upper=8.
        while residual(upper)<0:upper*=2
        for _ in range(50):
            mid=(lower+upper)/2
            if residual(mid)<0:lower=mid
            else:upper=mid
        distance=(lower+upper)/2
        supported_y=center.y-distance*COS
        height,part=surface(center.x,supported_y)
        target=lo.z+distance*SIN
        report=json.loads((worker/'inspection/refinement.json').read_text());packet_path=Path(report['source_packet']);packet=json.loads(packet_path.read_text())
        record=dict(asset_id=worker.name,previous_model_sha256=sha(worker/'model.blend'),bank_worker=str(bank),bank_model_sha256=bank_hash,
            previous_bounds=[list(lo),list(hi)],ray_translation=distance,world_translation=[0,-distance*COS,distance*SIN],
            support_point=[center.x,supported_y,height],support_part=part,minimum_elevation=target,
            clearance=.5,projection_delta=[0,(-(-distance*COS)*SIN-(distance*SIN)*COS)],
            limitation='Center support on actual bank surface; irregular lower foliage can intersect sloped ground and requires joint review.')
        packet['minimum_elevation']=target;packet['support_evidence']=str(OUT/f'understory-review/shrub-{index}-bank-support.json')
        write_json(packet_path,packet);write_json(Path(packet['support_evidence']),record);records.append(record)
    if sha(bank/'model.blend')!=bank_hash:raise ValueError('Bank changed during support solve')
    write_json(OUT/'understory-review/north-shrub-support-placement.json',dict(status='Projection-preserving placement solved; joint review pending',records=records))
    refine([65,66])
    if sha(bank/'model.blend')!=bank_hash:raise ValueError('Bank changed during shrub refinement')


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()

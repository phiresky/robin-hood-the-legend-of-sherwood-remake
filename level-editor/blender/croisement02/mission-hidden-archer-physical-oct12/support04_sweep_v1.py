"""Conservatively audit finite support tubes against qualified receiver alpha."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,collections
from pathlib import Path
import numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from restart14_butterfly07_anatomy_contacts import clip_planes
from restart406_alpha_contact_predicate import threshold_contact
from resource_guard import guard_reader
from render_slots import acquire,release
DEST=q.OUT/'geometry-v8-support04-sweep'

def main():
    assert not DEST.exists();guard_reader(reader,q.ROOT,DEST)
    input_path=q.OUT/'geometry-v8-short-support04/report.json'
    construction=json.loads(input_path.read_text())
    assert q.pin(q.ROOT/construction['model']['path'])==construction['model']
    qualification=json.loads((q.OUT/'qualification.json').read_text())
    units=[]
    for row in construction['rows']:
        if not row['status'].startswith('BUILT'):continue
        sx,sy=row['candidate']['screen'];radius=row['radius']
        start=np.array(row['support_start']);end=np.array(row['support_end'])
        ray=np.array([0,-reader.COS,reader.SIN]);lo=float(end@ray);hi=float(start@ray)
        assert lo<hi
        bounds=np.array([sx-radius,sy-radius,lo,sx+radius,sy+radius,hi])
        planes=np.array([[-1,0,0,bounds[0]],[0,-1,0,bounds[1]],[0,0,-1,bounds[2]],
                         [1,0,0,-bounds[3]],[0,1,0,-bounds[4]],[0,0,1,-bounds[5]]])
        units.append(dict(phase=row['phase'],component=row['component'],bounds=bounds,planes=planes,
            candidate=row['candidate'],counts=collections.Counter(),witnesses=[]))
    def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
        xyz=np.stack([tri[:,:,0],reader.SIN*tri[:,:,2]-reader.COS*tri[:,:,1],
                      reader.COS*tri[:,:,2]+reader.SIN*tri[:,:,1]],axis=2)
        uv=np.zeros((*tri.shape[:2],2))if uvs is None else uvs
        attrs=np.concatenate([xyz,uv],axis=2);low=xyz.min(1);high=xyz.max(1)
        opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE'
        image,sampler,factor=(None,{},1.)if opaque else texture_record(mat)
        cut=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
        for unit in units:
            b=unit['bounds'];indices=np.flatnonzero(np.all(high>=b[:3],axis=1)&np.all(low<=b[3:],axis=1))
            for ti in indices:
                clipped=clip_planes(attrs[ti],unit['planes'])
                if not len(clipped):continue
                decision=threshold_contact(clipped,image,sampler,factor,max(1e-12,cut-1e-8))
                if not decision['intersects']:continue
                target=unit['candidate']['target']
                target_node=placed['id']==target['asset']and node.get('name')==target['node']
                # Keep intended endpoint contacts separate, with measured depth extent.
                # This is evidence for visual review, not an automatic exemption.
                category='TARGET_WOOD_REQUIRES_JUNCTION_REVIEW'if target_node else 'OTHER_RECEIVER_CONTACT'
                unit['counts'][category]+=1
                unit['witnesses'].append(dict(category=category,asset=placed['id'],node=node.get('name'),
                    primitive_index=int(pi),triangle_index=int(ti),
                    depth_range=[float(clipped[:,2].min()),float(clipped[:,2].max())],
                    extent_ahead_of_target=float(clipped[:,2].max()-target['camera_depth']),
                    alpha_witness_value=decision['value'],alpha_decision_mode=decision['mode']))
    def finish(rays,records,mp):
        DEST.mkdir()
        rows=[dict(phase=u['phase'],component=u['component'],bounds=u['bounds'].tolist(),
                   counts=dict(u['counts']),witnesses=u['witnesses'])for u in units]
        (DEST/'report.json').write_text(json.dumps(dict(status='FINITE_SUPPORT_SWEEP_REQUIRES_REVIEW',
            construction=q.pin(input_path),map=q.pin(mp),records=records,rows=rows,
            limits=['Square prism circumscribes the radius0.2 twelve-sided stem; contacts are conservative.',
                    'Target wood contacts are recorded, never silently accepted.',
                    'Static alpha checked at level0; runtime mips can differ.',
                    'Source foliage bodies retain their separate unresolved depth/contact holds.']),indent=2)+'\n')
        print([{k:r[k]for k in ['phase','component','counts']}for r in rows],flush=True)
    rays=[dict(screen=u['candidate']['screen'],hits=[])for u in units]
    reader.main(ray_records=rays,triangle_callback=inspect,postprocess=finish,output=DEST,
                query_margin=200,minimum_free_bytes=10*1024**3,output_limit_bytes=16*1024**2,
                expected_map_sha256=qualification['map']['sha256'])

if __name__=='__main__':
    acquire(slots=2)
    try:main()
    finally:release()

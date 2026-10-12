"""Measure untested pixel-center attachment routes for retained foliage islands."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,math
from pathlib import Path
import numpy as np
from scipy.ndimage import label
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from resource_guard import guard_reader
from render_slots import acquire,release
DEST=q.OUT/'support04-missing-routes-v1'

def main():
    assert not DEST.exists();guard_reader(reader,q.ROOT,DEST)
    qualification=json.loads((q.OUT/'qualification.json').read_text());row=qualification['rows'][3]
    field=json.loads((q.OUT/'conformation-v5-local-cell-bounds/depth-field.json').read_text())['rows']['4']
    anchor=json.loads((q.OUT/'depth-proposal-v1.json').read_text())['4']
    records={}
    for phase,frame in zip(['initial','transition-0','transition-1','applied'],
             row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames']):
        mask=np.array(q.rgba(frame))[:,:,3]>0;labels,count=label(mask,np.ones((3,3)))
        sizes=np.bincount(labels.ravel());sizes[0]=0;main_component=int(sizes.argmax())
        for y,x in np.argwhere(mask):
            sx,sy=int(frame['top_left'][0]+x),int(frame['top_left'][1]+y);component=int(labels[y,x])
            missing_large_island=phase=='transition-1'and sizes[component]==21 or phase=='applied'and sizes[component]==31
            left_main=component==main_component and sx<=18
            if not(left_main or missing_large_island):continue
            records.setdefault((sx,sy),[]).append(dict(phase=phase,component=component,pixels=int(sizes[component])))
    def back_depth(x,y):
        xlo,xhi=row['source_union_box'][0],row['source_union_box'][2]
        def corner(sx,sy):
            t=(sx-xlo)/(xhi-xlo);wy=anchor['world_y_left']*(1-t)+anchor['world_y_right']*t+anchor['depth_bulge']*math.sin(math.pi*t)
            a,b,c,d=field['box'];advance=field['camera_ray_advance'][sy-b][sx-a]if a<=sx<=c and b<=sy<=d else 0
            return -wy/reader.COS-sy*reader.SIN/reader.COS+advance
        return(corner(x,y)+corner(x+1,y+1))*.5-anchor['thickness']
    rays=[dict(screen=[x+.5,y+.5],source_components=tags,back_depth=back_depth(x,y),hits=[])for(x,y),tags in sorted(records.items())]
    def finish(rays,receivers,mp):
        rows=[]
        for r in rays:
            behind=[h for h in r['hits']if h['passes_alpha_and_culling']and h['camera_depth']<r['back_depth']-.01]
            hit=behind[0]if behind else None
            wood=hit is not None and hit['asset']in {'croisement02-tree-24','croisement02-tree-25'}and ' wood 'in hit['node']and hit['front_facing_dot']>0
            rows.append(dict(screen=r['screen'],source_components=r['source_components'],leaf_back_depth=r['back_depth'],
                nearest_behind=hit,candidate=wood,gap=None if hit is None else r['back_depth']-hit['camera_depth']))
        DEST.mkdir();(DEST/'report.json').write_text(json.dumps(dict(status='SUPPLEMENTAL_CENTERLINE_EVIDENCE_NOT_SUPPORT_ACCEPTANCE',
            map=q.pin(mp),receivers=receivers,rows=rows,limits=[
                'Only previously under-sampled left main regions and21/31pixel islands.',
                'Nearest opaque receiver behind leaf back must be front-facing Tree24/25 wood.',
                'Finite tube sweep, saved-model source coverage and junction views required before acceptance.']),indent=2)+'\n')
        print(dict(rays=len(rows),wood_candidates=sum(r['candidate']for r in rows),short_candidates=sum(r['candidate']and r['gap']<=6 for r in rows)),flush=True)
    reader.main(ray_records=rays,postprocess=finish,output=DEST,minimum_free_bytes=10*1024**3,
        output_limit_bytes=16*1024**2,expected_map_sha256=qualification['map']['sha256'])

if __name__=='__main__':
    acquire(slots=2)
    try:main()
    finally:release()

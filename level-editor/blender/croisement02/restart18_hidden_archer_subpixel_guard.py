"""Supersampled CPU diagnostic for opaque inferred supports and attached leaves."""
import json
import numpy as np
from restart18_hidden_archer_route_cpu import BASE,RAY,SIN,RockDepth,screen,sha
from restart18_hidden_archer_compact_graph import unpack
from restart18_hidden_archer_support_guard import tube
DEST=BASE/'climbing-v17/compact-subpixel-cpu-v1'
def main():
    assert not DEST.exists();DEST.mkdir();fp=BASE/'climbing-v17/compact-fans-cpu-v1/report.json';fans=json.loads(fp.read_text());ep=BASE/'climbing-v17/exact-geometry-readonly-v2/report.json';original=json.loads(ep.read_text());records=[]
    for state in fans['states']:
        source=next(s for s in original['states'] if s['state']==state['state']);a={k:unpack(v) for k,v in source['arrays'].items()};vertices=a['world_vertices'];loops=a['polygon_loop_vertices'];offsets=a['polygon_offsets'];native=[];nativepixels=[]
        for k in range(source['native_pixels']):
            poly=vertices[loops[offsets[2*k]:offsets[2*k+1]]];native.extend([poly[[0,1,2]],poly[[0,2,3]]]);nativepixels.append(np.floor(screen(poly.mean(0))).astype(int))
        segments=state['candidate_segments']+state['offmap_continuations'];stemtri=np.concatenate([tube(np.array(s['start']),np.array(s['end']),s['radius']) for s in segments]);leaftri=np.concatenate([np.array(l['polygon'])[[[0,1,2],[0,2,3]]] for l in state['leaf_blades']]);trees=[RockDepth(stemtri),RockDepth(leaftri)];native_tree=RockDepth(np.array(native));pixels=np.array(nativepixels);points=np.concatenate([pixels+[dx,dy] for dy in [.125,.375,.625,.875] for dx in [.125,.375,.625,.875]]);native_depth=native_tree.front(points);assert np.isfinite(native_depth).all();rows=[]
        for role,tree in zip(['support','inferred leaves'],trees):
            front=tree.front(points);bad=np.flatnonzero(front>native_depth-1e-5);rows.append(dict(role=role,samples=len(points),native_subpixel_first_hit_displacements=len(bad),examples=[dict(source=points[i].tolist(),depth_excess=float(front[i]-native_depth[i])) for i in bad[:40]],maximum_depth_excess=float(np.max(front-native_depth))))
        known=set(map(tuple,pixels));lo=pixels.min(0)-1;hi=pixels.max(0)+2;empty=np.array([(x,y) for y in range(max(0,lo[1]),hi[1]) for x in range(lo[0],hi[0]) if (x,y) not in known]);points=np.concatenate([empty+[dx,dy] for dy in [.125,.375,.625,.875] for dx in [.125,.375,.625,.875]])
        for row,tree in zip(rows,trees):
            front=tree.front(points);bad=np.flatnonzero(np.isfinite(front));row['native_empty_subpixel_hits']=len(bad);row['native_empty_examples']=points[bad[:40]].tolist()
        records.append(dict(state=state['state'],roles=rows,status='PASS' if all(not r['native_subpixel_first_hit_displacements'] and not r['native_empty_subpixel_hits'] for r in rows) else 'SUBPIXEL HOLD'));print(json.dumps(records[-1]),flush=True)
    result=dict(status='CPU diagnostic; no model construction',inputs={str(fp):sha(fp),str(ep):sha(ep)},states=records,sampling='4x4 point grid per native source pixel; compares opaque inferred triangles against exact observed saved leaf faces, with native-empty cells tested separately.');(DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n')
if __name__=='__main__':main()

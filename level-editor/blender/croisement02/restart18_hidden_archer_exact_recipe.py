"""Reject inferred leaf geometry crossing any known source-plane boundary."""
import json
import numpy as np
from restart18_hidden_archer_route_cpu import BASE,sha
from restart18_hidden_archer_source_planes import SourcePlanes
from restart18_hidden_archer_support_guard import tube
DEST=BASE/'climbing-v17/compact-fans-cpu-v4'
def main():
    assert not DEST.exists();DEST.mkdir();fp=BASE/'climbing-v17/compact-fans-cpu-v3/report.json';old=json.loads(fp.read_text());ep=BASE/'climbing-v17/exact-geometry-readonly-v2/report.json';exact=json.loads(ep.read_text());states=[]
    for state in old['states']:
        original=next(s for s in exact['states'] if s['state']==state['state']);planes=SourcePlanes(original);problems=[]
        for i,s in enumerate(state['candidate_segments']+state['offmap_continuations']):
            for problem in planes.triangle_source_conflicts(tube(np.array(s['start']),np.array(s['end']),s['radius'])):problem['segment']=i;problems.append(problem)
        leaves=[];rejected=[]
        for i,leaf in enumerate(state['leaf_blades']):
            poly=np.array(leaf['polygon']);conflicts=planes.triangle_source_conflicts(poly[[[0,1,2],[0,2,3]]])
            if conflicts:rejected.append(dict(leaf=i,conflicts=conflicts))
            else:leaves.append(leaf)
        known=[p for p in problems if p['role']=='known-native'];empty=[p for p in problems if p['role']=='native-empty'];new=dict(state);new.update(leaf_blades=leaves,accepted_blades=len(leaves),exact_plane_rejected_leaf_blades=len(rejected),source_guard=dict(status='EXACT KNOWN SOURCE PLANES PASS' if not known else 'HOLD',known_conflicts=known,native_empty_conflicts=empty,native_empty_area_sum=float(sum(p['projected_triangle_area'] for p in empty)),area_sum_caveat='Triangle projected areas may overlap; this sum is a conservative overcount, not a union area.',method='Clip every opaque inferred triangle to every overlapping native cell and compare the two linear depth planes at every clipped vertex. All retained inferred leaf blades avoid both known foreground and empty native cells. Thin support continuation outside native cells is an explicit inferred exception.'));states.append(new);print(json.dumps(dict(state=state['state'],known=len(known),empty=len(empty),empty_area=new['source_guard']['native_empty_area_sum'],retained_leaves=len(leaves),rejected_leaves=len(rejected))),flush=True)
    result=dict(status='CPU EXACT KNOWN-SOURCE CONSTRUCTION RECIPE' if all(not s['source_guard']['known_conflicts'] for s in states) else 'EXACT KNOWN SOURCE HOLD',inputs={str(fp):sha(fp),str(ep):sha(ep),**old['inputs']},states=states);encoded=json.dumps(result,separators=(',',':'))+'\n';assert len(encoded.encode())<3*1024**2;(DEST/'report.json').write_text(encoded)
if __name__=='__main__':main()

"""CPU finite-radius and native-ray guards for compact support graph hypotheses."""
import json,math
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from restart18_hidden_archer_route_cpu import BASE,RockDepth,RAY,screen,sha
from restart18_hidden_archer_compact_graph import unpack
from restart14_hidden_archer_lobes_v16 import nearest_origin
DEST=BASE/'climbing-v17/compact-support-guard-v6'
class Receiver:
    def __init__(self,vertices,indices):
        self.tri=vertices[indices];self.c=self.tri.mean(1);self.radius=np.linalg.norm(self.tri-self.c[:,None],axis=2).max(1);self.tree=cKDTree(self.c);self.vertices=cKDTree(vertices);self.maxradius=float(self.radius.max())
    def distance(self,point):
        upper=self.vertices.query(point)[0];ids=self.tree.query_ball_point(point,float(upper)+self.maxradius+1e-6);tri=self.tri[ids];d,q=nearest_origin(tri-point);i=int(d.argmin());normal=np.cross(tri[i,1]-tri[i,0],tri[i,2]-tri[i,0]);normal/=np.linalg.norm(normal);return math.sqrt(d[i])*(1 if -q[i]@normal>=-1e-8 else -1)
def tube(p,q,radius=.12):
    direction=q-p;direction/=np.linalg.norm(direction);u=np.cross(direction,[0,0,1]);
    if np.linalg.norm(u)<.01:u=np.cross(direction,[1,0,0])
    u/=np.linalg.norm(u);v=np.cross(direction,u);circle=np.array([u*math.cos(j*math.tau/8)+v*math.sin(j*math.tau/8) for j in range(8)])*radius;a=p+circle;b=q+circle;faces=[a[::-1],b]+[np.array([a[j],a[(j+1)%8],b[(j+1)%8],b[j]]) for j in range(8)]
    return np.array([np.array([face[0],face[k],face[k+1]]) for face in faces for k in range(1,len(face)-1)])
def main():
    assert not DEST.exists();DEST.mkdir();sp=BASE/'surface-v8/surfaces.npz';d=np.load(sp);receivers=[Receiver(d[f'vertices{i}'],d[f'triangles{i}']) for i in [0,1]];gp=BASE/'climbing-v17/compact-support-cpu-v4/report.json';graph=json.loads(gp.read_text());ep=BASE/'climbing-v17/exact-geometry-readonly-v2/report.json';exact=json.loads(ep.read_text());records=[]
    def clear(point):return min(r.distance(point) for r in receivers)
    def segment_clear(p,q,r=.12):
        stack=[(p,q,clear(p),clear(q),0)];lower=math.inf;tests=0
        while stack:
            p,q,dp,dq,level=stack.pop();length=np.linalg.norm(q-p);bound=min(dp,dq)-length/2-r;tests+=1
            if bound>=.005:lower=min(lower,bound);continue
            if min(dp,dq)<r or level>=12:return False,min(bound,dp-r,dq-r),tests
            m=(p+q)/2;dm=clear(m);stack.extend([(p,m,dp,dm,level+1),(m,q,dm,dq,level+1)])
        return True,lower,tests
    for state in graph['states']:
        native=next(s for s in exact['states'] if s['state']==state['state']);a={k:unpack(v) for k,v in native['arrays'].items()};hits=a['native_first_hits'];shift=RAY*.6;nodes={n['index']:np.array(n['world'])-shift for n in state['nodes']};segments=[];failed=[];checks=[]
        for edge in state['edges']:
            p,q=[nodes[i] for i in edge];projected=screen(np.array([p,q]));pieces=[(p,q)];bridge=any(set(edge)==set(b) for b in state.get('tiny_island_bridges',[]))
            if bridge and abs(projected[1,0]-projected[0,0])<.1 and abs(projected[1,1]-projected[0,1])>1.5:
                middle=(p+q)/2+np.array([.4,0,0]);pieces=[(p,middle),(middle,q)]
            checked=[segment_clear(x,y,.05) for x,y in pieces];ok=all(c[0] for c in checked);bound=min(c[1] for c in checked);count=sum(c[2] for c in checked);checks.append(dict(edge=edge,passed=ok,clearance_lower_bound=bound,samples=count,tiny_island_bridge=bridge,explicit_subpixel_bend=len(pieces)>1))
            if ok:segments.extend((x,y,.05) for x,y in pieces)
            else:failed.append(edge)
        anchors=[]
        for anchor in state['anchors']:
            p=np.array(anchor['support'])-shift;hit=np.array(anchor['geometric_rock_hit']);gap=np.linalg.norm(p-hit);lo=0.;hi=gap
            # Root remains outside both receivers with a 0.2-unit center clearance;
            # physical radius0.12 leaves at most0.08 local contact clearance.
            for _ in range(24):
                middle=(lo+hi)/2
                if clear(hit+RAY*middle)<.2:lo=middle
                else:hi=middle
            q=hit+RAY*hi;ok,bound,count=segment_clear(p,q);anchors.append(dict(node=anchor['node'],from_world=p.tolist(),to_world=q.tolist(),passed=ok,clearance_lower_bound=bound,endpoint_clearance=clear(q),samples=count))
            if ok:segments.append((p,q,.12))
        triangles=np.concatenate([tube(p,q,r) for p,q,r in segments]);support_depth=RockDepth(triangles);source_xy=hits[:,:2]+np.array(native['source_top_left'])+.5;front=support_depth.front(source_xy);olddepth=hits[:,4:7]@RAY;occlusions=np.flatnonzero(front>olddepth-1e-4);px=np.floor(source_xy).astype(int);known={tuple(p) for p in px};lower=px.min(0)-2;upper=px.max(0)+3;extra_points=np.array([(x+.5,y+.5) for y in range(lower[1],upper[1]) for x in range(lower[0],upper[0]) if (x,y) not in known]);extra_hits=support_depth.front(extra_points);extra=np.flatnonzero(np.isfinite(extra_hits))
        records.append(dict(state=state['state'],inferred_graph_source_ray_retreat=shift.tolist(),accepted_branch_segments=len(segments),failed_edges=failed,branch_clearance=checks,anchors=anchors,known_native_pixels=len(hits),support_first_hit_displacements=len(occlusions),displaced_pixels=source_xy[occlusions].tolist(),new_native_empty_hits=len(extra),extra_pixels=extra_points[extra].tolist(),candidate_segments=[dict(start=p.tolist(),end=q.tolist(),radius=r) for p,q,r in segments],status='LOCAL GRAPH GEOMETRIC GUARD PASS' if not failed and all(r['passed'] for r in anchors) and not len(occlusions) and not len(extra) else 'HOLD',limitations=['Finite-radius branch proof is a conservative signed-distance/Lipschitz tube bound against frozen rock and bank surfaces.','Support native rays treat new tubes as opaque, so preservation is independent of source-alpha clipping.','Only the retained local graph is checked; unresolved source islands and omitted inferred lobes remain separate construction blockers.']))
    result=dict(status='PRIVATE LOCAL GRAPH TEST; no model created',inputs={str(sp):sha(sp),str(gp):sha(gp),str(ep):sha(ep)},states=records);(DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps([{k:s[k] for k in ['state','status','accepted_branch_segments','known_native_pixels','support_first_hit_displacements','new_native_empty_hits']} for s in records]))
if __name__=='__main__':main()

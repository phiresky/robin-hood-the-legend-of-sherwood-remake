"""Route inferred branches beneath adjacent exact native leaf planes."""
import json,math
import numpy as np
from restart18_hidden_archer_route_cpu import BASE,RAY,screen,sha
from restart18_hidden_archer_support_guard import Receiver
from restart18_hidden_archer_source_planes import SourcePlanes
DEST=BASE/'climbing-v17/compact-support-guard-v7'
def simplify(points,tolerance=.01):
    if len(points)<3:return points
    a,b=points[0],points[-1];v=b-a;t=np.clip((points-a)@v/max(v@v,1e-20),0,1);error=np.linalg.norm(points-a-t[:,None]*v,axis=1);i=int(error.argmax())
    if error[i]<=tolerance:return points[[0,-1]]
    return np.vstack([simplify(points[:i+1],tolerance)[:-1],simplify(points[i:],tolerance)])
def main():
    assert not DEST.exists();DEST.mkdir();gp=BASE/'climbing-v17/compact-support-guard-v6/report.json';old=json.loads(gp.read_text());ep=BASE/'climbing-v17/exact-geometry-readonly-v2/report.json';exact=json.loads(ep.read_text());sp=BASE/'surface-v8/surfaces.npz';d=np.load(sp);receivers=[Receiver(d[f'vertices{i}'],d[f'triangles{i}']) for i in [0,1]];states=[]
    def clear(p):return min(r.distance(p) for r in receivers)
    def check(a,b,r):
        stack=[(a,b,clear(a),clear(b),0)];minimum=math.inf
        while stack:
            a,b,da,db,level=stack.pop();length=np.linalg.norm(b-a);bound=min(da,db)-length/2-r
            if bound>.003:minimum=min(minimum,bound);continue
            if min(da,db)<r or level>=13:return False,min(bound,da-r,db-r)
            middle=(a+b)/2;dm=clear(middle);stack.extend([(a,middle,da,dm,level+1),(middle,b,dm,db,level+1)])
        return True,minimum
    for state in old['states']:
        original=next(s for s in exact['states'] if s['state']==state['state']);planes=SourcePlanes(original);new=[];failures=[];routes=[];maxretreat=0.
        for index,s in enumerate(state['candidate_segments']):
            a,b=np.array(s['start']),np.array(s['end']);radius=s['radius'];distance=np.linalg.norm(screen(b)-screen(a));count=max(2,int(math.ceil(distance/.05))+1);points=a+(b-a)*np.linspace(0,1,count)[:,None];projected=screen(points);depth=points@RAY;limit=np.array([planes.allowed_depth(p,radius+.025) for p in projected]);retreat=np.maximum(0,depth-limit+radius+.04);points-=retreat[:,None]*RAY;maxretreat=max(maxretreat,float(retreat.max()));points=simplify(points);pieces=[]
            for p,q in zip(points[:-1],points[1:]):
                if np.linalg.norm(q-p)<1e-8:continue
                ok,minimum=check(p,q,radius);entry=dict(start=p.tolist(),end=q.tolist(),radius=radius,clearance_lower_bound=minimum,original_segment=index);new.append(entry);pieces.append(len(new)-1)
                if not ok:failures.append(dict(segment=len(new)-1,clearance_lower_bound=minimum))
            routes.append(dict(original_segment=index,segments=pieces,maximum_retreat=float(retreat.max()),unchanged_projection_error=float(np.max(np.abs(screen(points)-screen(a+(b-a)*np.array([0.,1.])[:,None])))) if len(points)==2 else None))
        states.append(dict(state=state['state'],status='FINITE-RADIUS PASS; subpixel source audit pending' if not failures else 'HOLD',known_native_pixels=state['known_native_pixels'],candidate_segments=new,failed_segments=failures,original_routes=routes,maximum_inferred_source_ray_retreat=maxretreat,source_faces_unchanged=True,scope='Only inferred support bends change; local source projection and graph junctions retained. Each bend follows the lower adjacent native leaf plane rather than crossing in front of it.'))
        print(json.dumps(dict(state=state['state'],segments=len(new),failed=len(failures),max_retreat=maxretreat)),flush=True)
    result=dict(status='CPU inferred support revision',inputs={str(gp):sha(gp),str(ep):sha(ep),str(sp):sha(sp)},states=states);(DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n')
if __name__=='__main__':main()

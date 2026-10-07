"""Prepare attached inferred leaf blades around the cleared local branch network."""
import json,math
import numpy as np
from scipy.spatial import cKDTree
from restart18_hidden_archer_route_cpu import BASE,RAY,SIN,COS,RockDepth,screen,sha
from restart18_hidden_archer_compact_graph import unpack
from restart18_hidden_archer_support_guard import Receiver,tube
from restart18_hidden_archer_source_planes import SourcePlanes
DEST=BASE/'climbing-v17/compact-fans-cpu-v3'
def main():
    assert not DEST.exists();DEST.mkdir();gp=BASE/'climbing-v17/compact-support-guard-v7/report.json';graph=json.loads(gp.read_text());assert all(s['status']=='FINITE-RADIUS PASS; subpixel source audit pending' for s in graph['states']);ep=BASE/'climbing-v17/exact-geometry-readonly-v2/report.json';exact=json.loads(ep.read_text());sp=BASE/'surface-v8/surfaces.npz';d=np.load(sp);receivers=[Receiver(d[f'vertices{i}'],d[f'triangles{i}']) for i in [0,1]];states=[]
    def distance(p):return min(r.distance(p) for r in receivers)
    def triangle_clear(t):
        stack=[(t,0)];lower=math.inf
        while stack:
            t,level=stack.pop();center=t.mean(0);cover=np.linalg.norm(t-center,axis=1).max();bound=distance(center)-cover
            if bound>.01:lower=min(lower,bound);continue
            if level>=6:return False,bound
            lengths=[np.linalg.norm(t[(k+1)%3]-t[k]) for k in range(3)];k=int(np.argmax(lengths));j=(k+1)%3;l=(k+2)%3;mid=(t[k]+t[j])/2;stack.extend([(np.array([t[k],mid,t[l]]),level+1),(np.array([mid,t[j],t[l]]),level+1)])
        return True,lower
    for si,state in enumerate(graph['states']):
        original=next(s for s in exact['states'] if s['state']==state['state']);planes=SourcePlanes(original);hits=unpack(original['arrays']['native_first_hits']);xy=hits[:,:2]+original['source_top_left']+.5;known={tuple(np.floor(p).astype(int)):float(v) for p,v in zip(xy,hits[:,4:7]@RAY)};rng=np.random.default_rng(1818+si);segments=state['candidate_segments'];allpoints=np.concatenate([[s['start'],s['end']] for s in segments]);centers=[]
        for p in allpoints:
            if not centers or np.min(np.linalg.norm(np.array(centers)-p,axis=1))>2.8:centers.append(p)
        def native_clear(tri):return planes.triangles_clear(tri,4)
        # Short tapered continuation is anchored to selected existing top nodes.
        # It never retains the historical floating off-map lobes or long guides.
        offmap=[];tops=sorted([p for p in centers if 0<=screen(np.array(p))[1]<=1.5],key=lambda p:p[0]);chosen=[]
        for p in tops:
            if any(abs(p[0]-q[0])<10 for q in chosen):continue
            chosen.append(p)
            if len(chosen)==3:break
        for p in chosen:
            q=np.array(p)+np.array([0,SIN,COS])*5.5;tri=tube(np.array(p),q,.05);checks=[triangle_clear(t) for t in tri]
            if all(c[0] for c in checks) and native_clear(tri):
                offmap.append(dict(start=np.array(p).tolist(),end=q.tolist(),radius=.05));centers.extend([np.array(p)+np.array([0,SIN,COS])*t for t in [1.8,3.6,5.3]])
        leaves=[];rejected_native=0;rejected_surface=0;minimum=math.inf
        for ci,base in enumerate(centers):
            base=np.array(base);accepted_here=0
            for attempt in range(12):
                direction=-RAY*.75+rng.normal(0,.7,3);direction/=np.linalg.norm(direction);side=np.cross(direction,rng.normal(size=3));side/=np.linalg.norm(side);length=float(rng.uniform(.9,2.4));width=float(rng.uniform(.25,.55));tip=base+direction*length;mid=base+direction*length*.52;poly=np.array([base,mid+side*width,tip,mid-side*width]);tri=np.array([poly[[0,1,2]],poly[[0,2,3]]])
                if not native_clear(tri):rejected_native+=1;continue
                checks=[triangle_clear(t) for t in tri]
                if not all(c[0] for c in checks):rejected_surface+=1;continue
                leaves.append(dict(branch_anchor=base.tolist(),polygon=poly.tolist(),clearance_lower_bound=min(c[1] for c in checks),paired_back_ray_offset=.005,physical_connection='Leaf blade base is exactly on the finite-radius support centerline; opaque branch intersects its basal interior.'))
                minimum=min(minimum,min(c[1] for c in checks))
                accepted_here+=1
                if accepted_here>=3:break
        states.append(dict(state=state['state'],candidate_segments=segments,offmap_continuations=offmap,leaf_blades=leaves,cluster_centers=len(centers),accepted_blades=len(leaves),minimum_leaf_receiver_clearance=minimum,rejected_native_view=rejected_native,rejected_receiver=rejected_surface,native_pixels_preserved=state['known_native_pixels'],construction_recipe='Retain exact first2*N native polygons/material/UV/ownership. Remove every old inferred stem/fan. Add guarded finite-radius supports and these physically attached paired opaque inferred leaf blades. New materials remain explicitly inferred gray; no donor or generated texture.',limitations=['CPU native guard samples original pixel centers; supersampled silhouette and saved shader/first-hit checks remain required.','Every new leaf starts on a support; full morphology/contact must still be inspected in actual and solid8views.','Local crevice attachments are inferred, with endpoint clearance0.08world beyond tube radius; not observed roots.']))
    result=dict(status='CPU CONSTRUCTION RECIPE; saved-model review pending',inputs={str(gp):sha(gp),str(ep):sha(ep),str(sp):sha(sp)},states=states);encoded=json.dumps(result,separators=(',',':'))+'\n';assert len(encoded.encode())<3*1024**2,f'CPU recipe cap exceeded: {len(encoded.encode())}';(DEST/'report.json').write_text(encoded);print(json.dumps([{k:s[k] for k in ['state','cluster_centers','accepted_blades','minimum_leaf_receiver_clearance','rejected_native_view','rejected_receiver']} for s in states]))
if __name__=='__main__':main()

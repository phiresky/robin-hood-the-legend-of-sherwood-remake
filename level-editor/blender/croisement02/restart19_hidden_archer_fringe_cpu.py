"""Prepare a fuller connected fringe strictly beyond the native map boundary."""
import json,math
import numpy as np
from PIL import Image
from restart18_hidden_archer_route_cpu import BASE,RAY,SIN,COS,screen,sha
from restart18_hidden_archer_support_guard import Receiver,tube
DEST=BASE/'climbing-v18-compact/fringe-completion-cpu-v1'
def main():
    assert not DEST.exists();DEST.mkdir();fp=BASE/'climbing-v17/compact-fans-cpu-v4/report.json';old=json.loads(fp.read_text());sp=BASE/'surface-v8/surfaces.npz';d=np.load(sp);receivers=[Receiver(d[f'vertices{i}'],d[f'triangles{i}']) for i in [0,1]];up=np.array([0.,SIN,COS]);bindings={str(fp):sha(fp),str(sp):sha(sp)};states=[]
    def clear(p):return min(r.distance(p) for r in receivers)
    def triangle_clear(tri):
        stack=[(tri,0)];lower=math.inf
        while stack:
            tri,level=stack.pop();center=tri.mean(0);bound=clear(center)-np.linalg.norm(tri-center,axis=1).max()
            if bound>.01:lower=min(lower,bound);continue
            if level>=7:return False,bound
            k=int(np.argmax([np.linalg.norm(tri[(j+1)%3]-tri[j]) for j in range(3)]));j=(k+1)%3;l=(k+2)%3;mid=(tri[k]+tri[j])/2;stack.extend([(np.array([tri[k],mid,tri[l]]),level+1),(np.array([mid,tri[j],tri[l]]),level+1)])
        return True,lower
    for si,state in enumerate(old['states']):
        worker=BASE/f'climbing-v18-compact/profile-05-{state["state"]}';model=worker/'model.blend';construction=json.loads((worker/'construction.json').read_text());bindings[str(model)]=sha(model);assert bindings[str(model)]==construction['model_sha256'];src=np.array(Image.open(construction['source']).convert('RGBA'));xs=np.flatnonzero(src[0,:,3]>=128)+construction['source_top_left'][0];rng=np.random.default_rng(19019+si);existing=state['offmap_continuations'];segments=[];leaves=[];branch_records=[];targets=np.linspace(float(xs.min())+.2,float(xs.max())+.8,max(6,math.ceil((xs.max()-xs.min()+1)/4)));rejected=0
        for k,targetx in enumerate(targets):
            parent=min(range(len(existing)),key=lambda j:abs(existing[j]['start'][0]-targetx));stem=existing[parent];a,b=np.array(stem['start']),np.array(stem['end']);sy0,sy1=screen(np.array([a,b]))[:,1];base_y=-float(rng.uniform(.7,1.6));t=(base_y-sy0)/(sy1-sy0);assert 0<t<1;base=a+(b-a)*t;f=k/max(1,len(targets)-1);height=2.6+3.2*math.sin(math.pi*f)+float(rng.uniform(-.4,.4));target=base+np.array([targetx-base[0],0,0])+up*(height+base_y)+RAY*float(rng.uniform(-1.1,1.1));middle=(base+target)/2+up*float(rng.uniform(.2,.7));chain=np.array([base,middle,target]);branch=[]
            for p,q in zip(chain[:-1],chain[1:]):
                tri=tube(p,q,.05);checks=[triangle_clear(t) for t in tri]
                if screen(tri)[:,:,1].max()>=-.01 or not all(c[0] for c in checks):branch=[];break
                branch.append(dict(start=p.tolist(),end=q.tolist(),radius=.05,minimum_receiver_clearance=min(c[1] for c in checks),existing_stem_parent=parent))
            if not branch:rejected+=1;continue
            first=len(segments);segments+=branch;branch_records.append(dict(target_x=float(targetx),parent=parent,attachment_world=base.tolist(),attachment_on_existing_stem_t=float(t),segment_indices=list(range(first,len(segments)))))
            for seg in branch:
                p,q=np.array(seg['start']),np.array(seg['end']);length=np.linalg.norm(q-p);locations=np.linspace(0,1,max(2,math.ceil(length/.8)))
                for fraction in locations:
                    point=p+(q-p)*fraction;accepted=0
                    for attempt in range(20):
                        direction=up*.35-RAY*.3+rng.normal(0,.65,3);direction/=np.linalg.norm(direction);side=np.cross(direction,rng.normal(size=3));side/=np.linalg.norm(side);leaf_length=float(rng.uniform(1.1,2.6));width=float(rng.uniform(.35,.7));tip=point+direction*leaf_length;mid=point+direction*leaf_length*.52;poly=np.array([point,mid+side*width,tip,mid-side*width]);tri=poly[[[0,1,2],[0,2,3]]]
                        if screen(poly)[:,1].max()>=-.015:continue
                        checks=[triangle_clear(t) for t in tri]
                        if not all(c[0] for c in checks):continue
                        leaves.append(dict(polygon=poly.tolist(),branch_anchor=point.tolist(),paired_back_ray_offset=.005,minimum_receiver_clearance=min(c[1] for c in checks),branch_segment=first+branch.index(seg),fraction=float(fraction)));accepted+=1
                        if accepted>=5:break
        positions=np.concatenate([np.array(l['polygon']) for l in leaves]+[np.array([s['start'],s['end']]) for s in segments]);states.append(dict(state=state['state'],base_model=str(model),base_model_sha256=bindings[str(model)],native_cut_run_x=[int(xs.min()),int(xs.max())],existing_supports=existing,added_segments=segments,added_leaf_blades=leaves,branches=branch_records,rejected_branches=rejected,added_source_bounds=[screen(positions).min(0).tolist(),screen(positions).max(0).tolist()],status='ADDITIVE OFFMAP CPU RECIPE',proof='All new tube triangles and leaf vertices are strictly source_y<0. Paired backs shift only along source ray. Every branch root lies inside an existing opaque off-map stem, and every leaf base lies on its new finite-radius support. Existing entire v18 mesh/material/UV remains byte-equivalent.'))
        print(json.dumps(dict(state=state['state'],branches=len(branch_records),segments=len(segments),leaves=len(leaves),bounds=states[-1]['added_source_bounds'])),flush=True)
    result=dict(status='BOUNDED INFERRED-ONLY OFFMAP ADDITION; visual review pending',inputs=bindings,states=states,output_cap_bytes=3*1024**2,next_round_cap_bytes=32*1024**2,scope='Add a rounded, connected fringe across actual native cut runs; preserve all existing v18 geometry and known artwork. Gray inferred surfaces remain texture-incomplete.');encoded=json.dumps(result,separators=(',',':'))+'\n';assert len(encoded.encode())<3*1024**2;(DEST/'report.json').write_text(encoded);print(json.dumps(dict(bytes=len(encoded.encode()),sha256=sha(DEST/'report.json'))),flush=True)
if __name__=='__main__':main()

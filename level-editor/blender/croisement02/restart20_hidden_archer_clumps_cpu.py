"""Plan unequal connected off-map clumps, preserving the complete v18 baseline."""
import json, math
import numpy as np
from PIL import Image
from restart18_hidden_archer_route_cpu import BASE, RAY, SIN, COS, screen, sha
from restart18_hidden_archer_support_guard import Receiver, tube
DEST = BASE/'climbing-v18-compact/fringe-clumps-cpu-v3'

def main():
    assert not DEST.exists()
    DEST.mkdir()
    fp=BASE/'climbing-v17/compact-fans-cpu-v4/report.json'
    old=json.loads(fp.read_text())
    sp=BASE/'surface-v8/surfaces.npz'
    data=np.load(sp)
    receivers=[Receiver(data[f'vertices{i}'], data[f'triangles{i}']) for i in [0,1]]
    up=np.array([0.,SIN,COS]); right=np.array([1.,0.,0.])
    bindings={str(fp):sha(fp),str(sp):sha(sp)}; states=[]
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
        worker=BASE/f'climbing-v18-compact/profile-05-{state["state"]}'
        model=worker/'model.blend'; construction=json.loads((worker/'construction.json').read_text())
        bindings[str(model)]=sha(model)
        assert bindings[str(model)]==construction['model_sha256']
        source=np.array(Image.open(construction['source']).convert('RGBA'))
        xs=np.flatnonzero(source[0,:,3]>=128)+construction['source_top_left'][0]
        rng=np.random.default_rng(20031+si)
        existing=state['offmap_continuations']; segments=[]; leaves=[]; records=[]
        def add_segment(p,q,parent,role):
            triangles=tube(p,q,.05)
            if screen(triangles)[:,:,1].max()>=-.015:return None
            checks=[triangle_clear(t) for t in triangles]
            if not all(c[0] for c in checks):return None
            index=len(segments)
            segments.append(dict(start=p.tolist(),end=q.tolist(),radius=.05,minimum_receiver_clearance=min(c[1] for c in checks),existing_stem_parent=parent,role=role))
            return index
        def foliage(index,count):
            segment=segments[index]; p,q=np.array(segment['start']),np.array(segment['end'])
            for _ in range(count):
                fraction=float(rng.uniform(.03,1)); point=p+(q-p)*fraction
                direction=rng.normal(size=3)+up*.18
                direction/=np.linalg.norm(direction)
                side=np.cross(direction,rng.normal(size=3)); side/=np.linalg.norm(side)
                length=float(rng.uniform(1.5,3.6)); width=float(rng.uniform(.4,.85))
                tip=point+direction*length; middle=point+direction*length*.48
                polygon=np.array([point,middle+side*width,tip,middle-side*width])
                if screen(polygon)[:,1].max()>=-.025:continue
                checks=[triangle_clear(t) for t in polygon[[[0,1,2],[0,2,3]]]]
                if not all(c[0] for c in checks):continue
                leaves.append(dict(polygon=polygon.tolist(),branch_anchor=point.tolist(),paired_back_ray_offset=.005,minimum_receiver_clearance=min(c[1] for c in checks),branch_segment=index,fraction=fraction))
        # Unequal overlapping sampled volumes, never a rendered shell or repeated fork row.
        for ci,(fraction,height,width,vertical,depth,branches) in enumerate([
            (.13,3.3,5.4,2.7,3.3,26),(.47,6.2,7.1,5.1,4.5, 40),(.84,3.8,5.7,3.0,3.8,29)]):
            targetx=float(xs.min()+(xs.max()-xs.min())*fraction)
            parent=min(range(len(existing)),key=lambda j:abs(existing[j]['start'][0]-targetx))
            stem=existing[parent]; a,b=np.array(stem['start']),np.array(stem['end'])
            sy0,sy1=screen(np.array([a,b]))[:,1]
            base_y=-float(rng.uniform(.8,1.3)); t=(base_y-sy0)/(sy1-sy0)
            assert 0<t<1
            base=a+(b-a)*t
            center=base+right*(targetx-base[0])+up*(height+base_y)+RAY*[-1.4,1.2,-.6][ci]
            main_index=add_segment(base,center,parent,'Short attachment to unequal clump interior')
            assert main_index is not None
            foliage(main_index,65)
            child_indices=[]
            for _ in range(branches):
                vector=rng.normal(size=3); vector/=np.linalg.norm(vector)
                vector*=float(rng.uniform(.3,1))**(1/3)
                endpoint=center+right*vector[0]*width+up*vector[1]*vertical+RAY*vector[2]*depth
                if screen(endpoint)[1]>-.25:continue
                # Branch roots lie on the parent support, never in a floating point cloud.
                origin=base+(center-base)*float(rng.uniform(.5,1))
                index=add_segment(origin,endpoint,parent,'Irregular secondary branch within volumetric leaf clump')
                if index is None:continue
                child_indices.append(index);foliage(index,34)
            records.append(dict(clump=ci,center_world=center.tolist(),radii=[width,vertical,depth],parent=parent,attachment_world=base.tolist(),attachment_on_existing_stem_t=float(t),main_segment=main_index,child_segments=child_indices))
        positions=np.concatenate([np.array(l['polygon']) for l in leaves]+[np.array([s['start'],s['end']]) for s in segments])
        states.append(dict(state=state['state'],base_model=str(model),base_model_sha256=bindings[str(model)],native_cut_run_x=[int(xs.min()),int(xs.max())],existing_supports=existing,added_segments=segments,added_leaf_blades=leaves,clumps=records,added_source_bounds=[screen(positions).min(0).tolist(),screen(positions).max(0).tolist()],status='ADDITIVE OFFMAP CPU RECIPE',proof='Every finite support and leaf triangle has receiver clearance. Every leaf base lies on a support connected to an existing v18 off-map stem. Entire old v18 geometry is retained. All new geometry stays source_y<0.'))
        print(json.dumps(dict(state=state['state'],segments=len(segments),leaves=len(leaves),bounds=states[-1]['added_source_bounds'])),flush=True)
    result=dict(status='BOUNDED INFERRED-ONLY OFFMAP ADDITION; visual review pending',inputs=bindings,states=states,output_cap_bytes=3*1024**2,next_round_cap_bytes=32*1024**2,scope='Three unequal connected volumetric leaf clumps replace rejected row hypothesis, starting from unchanged v18. Inferred gray is explicitly texture-incomplete.')
    encoded=json.dumps(result,separators=(',',':'))+'\n'
    assert len(encoded.encode())<3*1024**2
    (DEST/'report.json').write_text(encoded)
    print(json.dumps(dict(bytes=len(encoded.encode()),sha256=sha(DEST/'report.json'))),flush=True)
if __name__=='__main__':main()

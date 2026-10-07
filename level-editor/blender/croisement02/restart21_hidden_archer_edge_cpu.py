"""Connect the existing off-map clump envelope to exact native cut runs."""
import json, math
import numpy as np
from PIL import Image
from restart18_hidden_archer_route_cpu import BASE, RAY, SIN, COS, screen, sha
from restart18_hidden_archer_support_guard import Receiver
DEST=BASE/'climbing-v18-compact/fringe-lower-leaves-cpu-v5'
def main():
    assert not DEST.exists(); DEST.mkdir()
    parent=BASE/'climbing-v18-compact/fringe-clumps-cpu-v3/report.json'
    old=json.loads(parent.read_text()); data=np.load(BASE/'surface-v8/surfaces.npz')
    receivers=[Receiver(data[f'vertices{i}'],data[f'triangles{i}']) for i in [0,1]]
    up=np.array([0.,SIN,COS]); right=np.array([1.,0.,0.]); states=[]
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
        record=json.loads((BASE/f'climbing-v18-compact/profile-05-{state["state"]}/construction.json').read_text())
        src=np.array(Image.open(record['source']).convert('RGBA'))
        xs=np.flatnonzero(src[0,:,3]>=128)+record['source_top_left'][0]
        segments=np.array([[v['start'],v['end']] for v in state['added_segments']]); xy=screen(segments)
        delta=xy[:,1]-xy[:,0]; length=np.sum(delta*delta,axis=1); rng=np.random.default_rng(21041+si); leaves=[]
        for x in xs:
            target=np.array([x+.5,-.1]); fractions=np.clip(np.sum((target-xy[:,0])*delta,axis=1)/length,0,1)
            nearest=xy[:,0]+delta*fractions[:,None]; index=int(np.argmin(np.linalg.norm(nearest-target,axis=1)))
            fraction=float(fractions[index]); base=segments[index,0]+(segments[index,1]-segments[index,0])*fraction
            center=base+right*(target[0]-base[0])+up*(screen(base)[1]+.012)+RAY*float(rng.uniform(-.45,-.1))
            mid=(base+center)/2; shoulder=float(rng.uniform(1.05,1.3)); width=float(rng.uniform(.85,1.1))
            poly=np.array([base,mid+right*shoulder,center+right*width,center-right*width,mid-right*shoulder])
            triangles=np.array([[poly[0],poly[k],poly[k+1]] for k in range(1,len(poly)-1)])
            checks=[triangle_clear(t) for t in triangles]
            assert all(c[0] for c in checks) and screen(poly)[:,1].max()<-.01
            leaves.append(dict(polygon=poly.tolist(),branch_anchor=base.tolist(),paired_back_ray_offset=.005,minimum_receiver_clearance=min(c[1] for c in checks),branch_segment=index,fraction=fraction,native_cut_column=int(x),role='Short attached leaf continues existing native edge; only off-map half is inferred.'))
        polygons=[screen(np.array(v['polygon'])) for v in state['added_leaf_blades']+leaves]
        coverage=[]
        for y in [-.25,-.1,-.02]:
            intervals=[]
            for poly in polygons:
                intersections=[]
                for a,b in zip(poly,np.roll(poly,-1,axis=0)):
                    if (a[1]<=y<b[1]) or (b[1]<=y<a[1]):intersections.append(float(a[0]+(b[0]-a[0])*(y-a[1])/(b[1]-a[1])))
                if len(intersections)>=2:intervals.append((min(intersections),max(intersections)))
            # Dense subpixel union test across entire exact opaque cut cells, not only centers.
            samples=np.concatenate([x+(np.arange(100)+.5)/100 for x in xs])
            missed=[float(x) for x in samples if not any(a<=x<=b for a,b in intervals)]
            coverage.append(dict(source_y=y,samples=len(samples),missed=missed));assert not missed
        states.append(dict(state=state['state'],added_leaf_blades=leaves,near_edge_coverage=coverage,native_cut_columns=xs.tolist(),native_below_union='Exact retained observed cell faces cover source_y>=0 at these cells; all additions end at y=-0.012, a maximum 0.012-source-pixel geometric seam. All native centers and UV/RGB are rechecked after saving.'))
        print(json.dumps(dict(state=state['state'],leaves=len(leaves),coverage=coverage)),flush=True)
    result=dict(status='BOUNDED INFERRED-ONLY OFFMAP ADDITION; visual review pending',parent_packet=str(parent),parent_packet_sha256=sha(parent),inputs={str(parent):sha(parent)},states=states,scope='Retain exact v20 irregular clumps and all v18 geometry. Add only short attached lower leaf continuations across native cut runs.')
    (DEST/'report.json').write_text(json.dumps(result,separators=(',',':'))+'\n');print(sha(DEST/'report.json'))
if __name__=='__main__':main()

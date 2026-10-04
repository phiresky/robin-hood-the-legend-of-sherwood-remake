"""Infer rooted leaf paths from a native grass silhouette without changing alpha."""
import heapq,json,math,argparse
from pathlib import Path
import numpy as np
from scipy.ndimage import binary_closing,binary_dilation,distance_transform_edt
from PIL import Image
from catalog import OUT


def skeletonize(binary):
    a=np.pad(binary.astype(np.uint8),1)
    while True:
        changes=0
        for step in [0,1]:
            p=[a[:-2,1:-1],a[:-2,2:],a[1:-1,2:],a[2:,2:],a[2:,1:-1],a[2:,:-2],a[1:-1,:-2],a[:-2,:-2]]
            neighbors=sum(p);transitions=sum((p[i]==0)&(p[(i+1)%8]>0) for i in range(8))
            gate=(p[0]*p[2]*p[4]==0)&(p[2]*p[4]*p[6]==0) if step==0 else (p[0]*p[2]*p[6]==0)&(p[0]*p[4]*p[6]==0)
            remove=(a[1:-1,1:-1]>0)&(neighbors>=2)&(neighbors<=6)&(transitions==1)&gate
            changes+=int(remove.sum());a[1:-1,1:-1][remove]=0
        if not changes:return a[1:-1,1:-1]>0


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--mask',type=int,default=80);parser.add_argument('--revision',type=int,choices=[1,2,3],default=1);args=parser.parse_args()
    row=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==args.mask)
    mask=np.array(Image.open(OUT/'baseline/masks'/row['png']).convert('L'))>127
    yy,xx=np.nonzero(mask);cx=(xx.min()+xx.max()+1)/2;root_y=yy.max()+1-(yy.max()-yy.min()+1)*.18
    # Connectivity aid only: the native alpha remains immutable.
    skeleton=skeletonize(mask if args.revision==3 else binary_closing(binary_dilation(mask,iterations=1),iterations=1))
    bridges={}
    if args.revision==3:
        from scipy.ndimage import label
        components,total=label(skeleton,np.ones((3,3)))
        for y,x in zip(*np.nonzero(skeleton)):
            neighbors=[]
            for dy in range(-4,5):
                for dx in range(-4,5):
                    yy,xx=y+dy,x+dx
                    if 0<=yy<mask.shape[0] and 0<=xx<mask.shape[1] and components[yy,xx] and components[yy,xx]!=components[y,x] and math.hypot(dx,dy)<=4:
                        neighbors.append(((yy,xx),math.hypot(dx,dy)*3))
            if neighbors:bridges[(y,x)]=neighbors
    coords=list(zip(*np.nonzero(skeleton)));nodes=set(coords)
    root=min(nodes,key=lambda p:(p[0]-root_y)**2+(p[1]-cx)**2)
    distances={root:0.};parents={};queue=[(0.,root)]
    while queue:
        cost,p=heapq.heappop(queue)
        if cost!=distances[p]:continue
        for q,weight in bridges.get(p,[]):
            nc=cost+weight
            if nc<distances.get(q,float('inf')):distances[q]=nc;parents[q]=p;heapq.heappush(queue,(nc,q))
        for dy in [-1,0,1]:
            for dx in [-1,0,1]:
                q=(p[0]+dy,p[1]+dx)
                if (not dx and not dy) or q not in nodes:continue
                nc=cost+math.hypot(dx,dy)
                if nc<distances.get(q,float('inf')):distances[q]=nc;parents[q]=p;heapq.heappush(queue,(nc,q))
    endpoints=[]
    for p in distances:
        neighbors=sum((p[0]+dy,p[1]+dx) in nodes for dy in [-1,0,1] for dx in [-1,0,1] if dx or dy)
        if neighbors==1 and math.hypot(p[0]-root_y,p[1]-cx)>12:endpoints.append(p)
    endpoints.sort(key=lambda p:math.atan2(root_y-p[0],p[1]-cx))
    if len(endpoints)<5:raise ValueError(f'Only {len(endpoints)} rooted leaf tips; inspect the structural guide')
    paths=[];fields=[]
    for endpoint in endpoints:
        path=[endpoint]
        while path[-1]!=root:path.append(parents[path[-1]])
        paths.append(path)
        seeds=np.ones(mask.shape,bool)
        for y,x in path:seeds[y,x]=False
        field=distance_transform_edt(seeds)
        if args.revision>=2:
            gy,gx=np.indices(mask.shape);angle=np.arctan2(root_y-gy,gx-cx)
            direction=math.atan2(root_y-endpoint[0],endpoint[1]-cx)
            delta=np.abs(np.arctan2(np.sin(angle-direction),np.cos(angle-direction)))
            field=field+.05*np.hypot(gx-cx,gy-root_y)*delta
        fields.append(field)
    labels=np.argmin(np.stack(fields),axis=0).astype(int);labels[~mask]=-1
    directory=OUT/f'grass-{args.mask:03}-leaf-paths-v{args.revision}';directory.mkdir(exist_ok=True)
    if (directory/'paths.json').exists():raise FileExistsError(directory)
    rng=np.random.default_rng(9000+args.mask);colors=rng.integers(60,255,(len(paths),3),dtype=np.uint8)
    picture=np.zeros((*mask.shape,3),np.uint8);picture[mask]=colors[labels[mask]]
    Image.fromarray(picture).resize((mask.shape[1]*8,mask.shape[0]*8),Image.Resampling.NEAREST).save(directory/'leaf-assignment.png')
    report=dict(status='inferred leaf path assignment; native source alpha unchanged',native_mask=args.mask,native_bbox=row['box_top_left']+row['box_size'],root_source_local=[cx,root_y],
                traced_root_pixel=[int(v) for v in root],leaf_count=len(paths),rooted_skeleton_pixels=len(distances),skeleton_pixels=len(nodes),
                paths=[[[int(x),int(y)] for y,x in path] for path in paths],
                observed_pixels=[dict(x=int(x),y=int(y),leaf=int(labels[y,x])) for y,x in zip(*np.nonzero(mask))])
    (directory/'paths.json').write_text(json.dumps(report,indent=2)+'\n');print(directory)

if __name__=='__main__':main()

"""Extract native log silhouettes and editable thin-branch centerlines."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt,binary_closing,label
from catalog import OUT


def skeletonize(mask):
    work=np.pad(mask,1).astype(bool)
    while True:
        removed=0
        for phase in (0,1):
            p=[work[:-2,1:-1],work[:-2,2:],work[1:-1,2:],work[2:,2:],work[2:,1:-1],work[2:,:-2],work[1:-1,:-2],work[:-2,:-2]]
            neighbours=sum(v.astype('uint8') for v in p)
            transitions=sum((~p[i]&p[(i+1)%8]).astype('uint8') for i in range(8))
            candidate=work[1:-1,1:-1]&(neighbours>=2)&(neighbours<=6)&(transitions==1)
            if phase==0:candidate&=~(p[0]&p[2]&p[4])&~(p[2]&p[4]&p[6])
            else:candidate&=~(p[0]&p[2]&p[6])&~(p[0]&p[4]&p[6])
            removed+=int(candidate.sum());work[1:-1,1:-1][candidate]=False
        if not removed:return work[1:-1,1:-1]


def main():
    from hashlib import sha256
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());profiles=[]
    record=level['masks'][102];x0,y0=record['box_top_left'];mask=np.asarray(Image.open(OUT/'baseline/masks/000102.png').convert('L'))>0
    for x in range(0,mask.shape[1],3):
        yy=np.flatnonzero(mask[:,x])
        if not len(yy):continue
        profiles.append([x+x0+.5,float((yy.min()+yy.max())/2+y0+.5),float((yy.max()-yy.min()+1)/2)])
    for i,p in enumerate(profiles):
        a=profiles[max(0,i-1)];b=profiles[min(len(profiles)-1,i+1)];slope=(b[1]-a[1])/max(b[0]-a[0],1)
        p[2]=p[2]/np.sqrt(1+slope*slope)+.45
    record=level['masks'][103];x0,y0=record['box_top_left'];mask=np.asarray(Image.open(OUT/'baseline/masks/000103.png').convert('L'))>0
    yy,xx=np.indices(mask.shape);thin=mask&(xx+x0>=190)&(yy+y0<1048)
    connected=binary_closing(thin,structure=np.ones((3,3),bool))
    components,count=label(connected,np.ones((3,3),int))
    for component in range(1,count+1):
        if np.count_nonzero(components==component)<6:connected[components==component]=False
    dist=distance_transform_edt(mask);skeleton=skeletonize(connected);points={tuple(p) for p in np.argwhere(skeleton)}
    graph={p:[(p[0]+dy,p[1]+dx) for dy in (-1,0,1) for dx in (-1,0,1) if (dy or dx) and (p[0]+dy,p[1]+dx) in points and not (dy and dx and ((p[0]+dy,p[1]) in points or (p[0],p[1]+dx) in points))] for p in points}
    nodes={p for p,v in graph.items() if len(v)!=2};seen=set();paths=[]
    for start in sorted(nodes):
        for neighbour in graph[start]:
            edge=tuple(sorted((start,neighbour)))
            if edge in seen:continue
            seen.add(edge);path=[start,neighbour];prior,current=start,neighbour
            while current not in nodes:
                after=next(p for p in graph[current] if p!=prior);seen.add(tuple(sorted((current,after))));path.append(after);prior,current=current,after
            if len(path)<2:continue
            simplified=path[::2]
            if simplified[-1]!=path[-1]:simplified.append(path[-1])
            paths.append([[float(x+x0+.5),float(y+y0+.5),float(max(.4,min(1.2,dist[y,x]*.65)))] for y,x in simplified])
    result=dict(source_sha256=sha256((OUT/'animation-references/composite-frame-0.png').read_bytes()).hexdigest(),mask_sha256={str(i):sha256((OUT/f'baseline/masks/{i:06}.png').read_bytes()).hexdigest() for i in (102,103)},foreground_log_profile=profiles,branches=paths,method='Native102 column silhouette with rounded hidden depth; native103 thin upper wood thinned into source-coordinate branch centerlines. These are geometry construction targets, not independent depth proof.')
    output=OUT/'southwest-log-revision/native-traces.json';output.parent.mkdir(exist_ok=True);output.write_text(json.dumps(result,indent=2)+'\n');print(len(profiles),'profile rings;',len(paths),'branch strokes;',output)

if __name__=='__main__':main()

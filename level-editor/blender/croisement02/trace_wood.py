"""Extract editable centreline candidates from inspected native wood silhouettes.

Run with NumPy, SciPy, scikit-image and Pillow. These are construction targets,
not independent proof of a branch's hidden depth or topology.
"""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from skimage.morphology import skeletonize
from catalog import OUT, TREES


def trace(index, alpha_override=None):
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    m=level['masks'][index];x,y=m['box_top_left']
    alpha=np.asarray(Image.open(OUT/f'baseline/masks/{index:06}.png').convert('L'))>0
    if alpha_override is not None:
        if alpha_override.shape != alpha.shape:raise ValueError('Replacement wood domain has different bounds')
        alpha=alpha_override
    distance=distance_transform_edt(np.pad(alpha,1))[1:-1,1:-1]
    sk=skeletonize(alpha);points={tuple(p) for p in np.argwhere(sk)}
    graph={p:[(p[0]+dy,p[1]+dx) for dy in (-1,0,1) for dx in (-1,0,1)
              if (dy or dx) and (p[0]+dy,p[1]+dx) in points] for p in points}
    nodes={p for p,v in graph.items() if len(v)!=2};edges=set();paths=[]
    for start in sorted(nodes):
        for neighbor in graph[start]:
            edge=tuple(sorted((start,neighbor)))
            if edge in edges:continue
            edges.add(edge);path=[start,neighbor];previous,current=start,neighbor
            while current not in nodes:
                after=next(p for p in graph[current] if p!=previous)
                edges.add(tuple(sorted((current,after))));path.append(after);previous,current=current,after
            length=sum(float(np.linalg.norm(np.subtract(b,a))) for a,b in zip(path,path[1:]))
            if length<7:continue
            simplified=[path[0]];traveled=0.
            for a,b in zip(path,path[1:]):
                traveled+=float(np.linalg.norm(np.subtract(b,a)))
                if traveled>=5:simplified.append(b);traveled=0.
            if simplified[-1]!=path[-1]:simplified.append(path[-1])
            paths.append([[float(p[1]+x+.5),float(p[0]+y+.5),float(max(.65,distance[p]))] for p in simplified])
    return dict(mask=index,parts=TREES[index],paths=paths,mask_box=[x,y,*m['box_size']],
                method='Native bitmap skeleton centreline with distance-to-edge radii; branch depth and cross-sections inferred')


def main():
    output=OUT/'wood-traces.json'
    output.write_text(json.dumps([trace(i) for i in TREES],indent=2)+'\n')
    print('Traced',len(TREES),'tree masks')

if __name__=='__main__':main()

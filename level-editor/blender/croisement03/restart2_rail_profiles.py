"""Trace solid rail silhouettes; retain real apertures and bracket profiles."""
import json,hashlib
from pathlib import Path
from PIL import Image
import numpy as np
from shapely.geometry import box,Polygon
from shapely.ops import unary_union
from shapely import constrained_delaunay_triangles
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement03-refinement/baseline'
OUT=BASE.parent/'restart2/rail-profiles-v1'

def main():
    OUT.mkdir(parents=True,exist_ok=False)
    masks=json.loads((BASE/'masks/manifest.json').read_text());result=[]
    for index in [110,111,112]:
        record=masks['masks'][index];file=BASE/'masks'/record['png'];bitmap=np.array(Image.open(file))>0;ox,oy=record['box_top_left'];rects=[]
        for y,row in enumerate(bitmap):
            xs=np.flatnonzero(row)
            if not len(xs):continue
            cuts=np.split(xs,np.where(np.diff(xs)>1)[0]+1)
            rects.extend(box(ox+int(run[0]),oy+y,ox+int(run[-1])+1,oy+y+1) for run in cuts)
        geometry=unary_union(rects).simplify(.55,preserve_topology=True)
        polygons=[geometry] if isinstance(geometry,Polygon) else list(geometry.geoms)
        components=[]
        for polygon in polygons:
            if polygon.area<1:continue
            triangles=constrained_delaunay_triangles(polygon)
            components.append(dict(area=polygon.area,rings=[list(polygon.exterior.coords)[:-1]]+[list(r.coords)[:-1] for r in polygon.interiors],triangles=[list(t.exterior.coords)[:-1] for t in triangles.geoms]))
        result.append(dict(mask=index,source_sha256=hashlib.sha256(file.read_bytes()).hexdigest(),components=components,source_pixels=int(bitmap.sum()),simplification_tolerance=.55))
    (OUT/'profiles.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['mask'],len(r['components'])) for r in result])

if __name__=='__main__':main()

"""Create closed source-owned patch domains; empty alpha never becomes a receiver slab."""
import argparse,json,pathlib
import numpy as np
from PIL import Image
from shapely.geometry import box
from shapely.ops import unary_union
from shapely import constrained_delaunay_triangles
R=pathlib.Path(__file__).resolve().parents[3];O=R/'level-editor/work/croisement01-refinement/mission-net01-oct12-v1'
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--separate-rope',action='store_true');args=ap.parse_args()
 s=json.loads((O/'source.json').read_text())['sources'];specs=[('initial-ground','initial',270,325),('applied-ground','g',0,105),('initial-weight','initial',0,61),('e-weight','e',82,118),('i-weight','i',82,118)];result={}
 for name,key,y0,y1 in specs:
  a=np.array(Image.open(s[key]['path']).convert('RGBA'))[:,:,3]>0;selected=np.zeros_like(a);selected[y0:y1]=a[y0:y1];rectangles=[]
  if args.separate_rope and name=='initial-weight':
   for y in range(48,61):
    xs=np.where(selected[y] & (np.arange(selected.shape[1])>=58))[0]
    runs=np.split(xs,np.where(np.diff(xs)>1)[0]+1);rope=max(runs,key=len);assert len(rope)>=2;selected[y,rope]=False
  for y,row in enumerate(selected):
   x=0
   while x<len(row):
    if not row[x]:x+=1;continue
    begin=x
    while x<len(row) and row[x]:x+=1
    rectangles.append(box(begin,y,x,y+1))
  domain=unary_union(rectangles).buffer(.01,join_style=2);polys=list(domain.geoms) if hasattr(domain,'geoms') else [domain];parts=[]
  for poly in polys:
   # Preserve pixel boundaries with the declared tiny expansion; respect all holes.
   if poly.area<.5:continue
   poly=poly.simplify(0,preserve_topology=True)
   triangulated=list(constrained_delaunay_triangles(poly).geoms)
   assert abs(sum(t.area for t in triangulated)-poly.area)<1e-7
   triangles=[list(t.exterior.coords)[:3] for t in triangulated]
   parts.append({'exterior':list(poly.exterior.coords)[:-1],'holes':[list(r.coords)[:-1] for r in poly.interiors],'triangles':triangles,'area':poly.area})
  result[name]={'source':key,'pixel_count':int(selected.sum()),'boundary_inflation_pixels':.01,'modeled_area':sum(x['area']for x in parts),'parts':parts,'scope':'Observed alpha domain with explicit0.01pixel boundary expansion to resolve diagonal point-touch topology; source texture alpha unchanged.'}
 (O/('domains-rope-separated-v1.json' if args.separate_rope else 'domains.json')).write_text(json.dumps(result)+'\n');print({k:(len(v['parts']),v['pixel_count'])for k,v in result.items()})
if __name__=='__main__':main()

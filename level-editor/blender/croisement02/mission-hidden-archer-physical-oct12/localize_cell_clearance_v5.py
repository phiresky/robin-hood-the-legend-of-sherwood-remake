"""Keep the existing depth field unchanged outside newly measured contact neighborhoods."""
import json
from pathlib import Path
import numpy as np
from scipy.ndimage import maximum_filter,gaussian_filter
import qualify as q

def main():
 source=q.OUT/'conformation-v4-cell-bounds/depth-field.json';data=json.loads(source.read_text());previous=q.OUT/'conformation-v2/depth-field.json';old=json.loads(previous.read_text());dest=q.OUT/'conformation-v5-local-cell-bounds';assert not dest.exists()
 for key,row in data['rows'].items():
  baseline=np.array(old['rows'][key]['camera_ray_advance']);seed=np.zeros_like(baseline);box=row['box']
  for cell in data['cell_bounds']:
   if str(cell['profile'])!=key:continue
   x,y=cell['cell']
   for dx,dy in [(0,0),(1,0),(1,1),(0,1)]:seed[y+dy-box[1],x+dx-box[0]]=max(seed[y+dy-box[1],x+dx-box[0]],cell['advance'])
  inferred=np.maximum(seed,gaussian_filter(maximum_filter(seed,size=5),1.0));field=np.maximum(baseline,inferred)
  row.update(camera_ray_advance=field.tolist(),maximum_advance=float(field.max()),maximum_added_advance=float((field-baseline).max()),changed_grid_corners=int((abs(field-baseline)>1e-8).sum()))
 data['cell_bound_source']=q.pin(source);data['localization']='Smooth only new contact requirements; do not repeatedly dilate the previous field.'
 dest.mkdir();(dest/'depth-field.json').write_text(json.dumps(data,indent=2)+'\n')
 print({k:{'added':r['maximum_added_advance'],'changed':r['changed_grid_corners']}for k,r in data['rows'].items()})
if __name__=='__main__':main()

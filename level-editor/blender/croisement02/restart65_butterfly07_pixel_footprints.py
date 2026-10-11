"""Separate center-ray misses from partially intersected source pixel footprints."""
import json
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull
from matplotlib.path import Path as Polygon
from restart64_butterfly07_source_ownership import ROOT,prepare,WING,geometry,sha
FIT=ROOT/'level-editor/work/croisement02-refinement/restart65-butterfly07-bright-pose-refinement-v1/fit.json'
OUT=FIT.parent/'pixel-footprints.json'
def main():
 assert not OUT.exists();fit=json.loads(FIT.read_text());records=[];summary=dict(missing_centers=0,bright_missing_centers=0,no_sample_coverage=0,bright_no_sample_coverage=0,partial_footprints=0)
 u=(np.arange(16)+.5)/16-.5;gy,gx=np.meshgrid(u,u);offsets=np.c_[gx.ravel(),gy.ravel()]
 for row in fit['rows']:
  c=prepare(row);body,_=geometry(row['parameters']);xy=body[:,:2]+c['shift'];shapes=[Polygon(xy[ConvexHull(xy).vertices])]
  for m in c['maps']:shapes.append(Polygon(WING[:,:2]@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift']))
  hit=np.logical_or.reduce([p.contains_points(c['q']) for p in shapes]);ids=np.flatnonzero(c['target']&~hit)
  for i in ids:
   points=c['q'][i]+offsets;coverage=np.logical_or.reduce([p.contains_points(points) for p in shapes]).mean();bright=bool(c['bright'][i]);summary['missing_centers']+=1;summary['bright_missing_centers']+=bright;summary['no_sample_coverage']+=coverage==0;summary['bright_no_sample_coverage']+=bright and coverage==0;summary['partial_footprints']+=coverage>0
   records.append(dict(phase=row['phase'],pixel=np.floor(c['q'][i]).astype(int).tolist(),bright=bright,geometry_coverage_16x16=float(coverage)))
 report=dict(status='PIXEL_FOOTPRINT_DIAGNOSTIC_NOT_EXACT_RGBA_REPRODUCTION',fit_sha256=sha(FIT),recipe_sha256=sha(Path(__file__)),summary={k:int(v) for k,v in summary.items()},records=records,limits=['16x16 footprint samples distinguish center misses from partial edge coverage, not exact intersection area.','Rendered RGB mixes source shading and antialiasing; source opacity is not proof of full physical pixel coverage.','Zero sampled coverage remains unexplained; do not silently discard those pixels.','No geometry, source bytes, motion or materials modified.'])
 OUT.write_text(json.dumps(report,indent=2)+'\n');print(summary)
if __name__=='__main__':main()

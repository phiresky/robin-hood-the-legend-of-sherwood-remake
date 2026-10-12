"""Infer an irregular sparse front transition beyond the map, retaining full back volume."""
import json,hashlib,shutil,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'tree01-native-transition-cpu-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not O.exists();O.mkdir();source=B/'tree01-canopy-volume-cpu-v6';d=json.loads((source/'geometry.json').read_text());v=np.array(d['vertices']);original=d['inferred_back_faces'];assert original==len(d['clusters'])*6;faces=d['faces'][:original];domain=np.array(Image.open(B/'tree01-native-fragment-v1/spatial-proposal.png'))>0;rng=np.random.default_rng(31012677);kept=[];rejected_boundary=[]
 for cluster,row in enumerate(d['clusters']):
  x,y,z=row['center'];sy=-y*S-z*C;outside=max(-x,-sy,0.)
  if outside<=0:continue
  # No straight clipping plane at the image boundary: entire leaf triangles
  # continue naturally, with density inferred to thin near observed branches.
  probability=.12+.88*min(1.,outside/60.)**1.4
  if rng.random()>probability:continue
  ids=list(range(original+cluster*6,original+(cluster+1)*6));accepted=[]
  for index in ids:
   face=d['faces'][index];p=v[face];native=np.column_stack((p[:,0],-p[:,1]*S-p[:,2]*C));okay=True
   for px,py in native:
    if px>=0 and py>=0:
     if px>=175 or py>=175 or not domain[min(174,int(py)),min(174,int(px))]:okay=False
   if okay:accepted.append(face)
   else:rejected_boundary.append(index)
  faces.extend(accepted);kept.append({'cluster':cluster,'native_center':[x,sy],'outside_distance':outside,'probability':probability,'accepted_faces':len(accepted)})
 used=sorted({i for f in faces for i in f});remap={i:j for j,i in enumerate(used)};d['vertices']=[d['vertices'][i]for i in used];d['uvs']=[d['uvs'][i]for i in used];d['faces']=[[remap[i]for i in f]for f in faces];p=np.array(d['vertices']);d['width']=float(np.ptp(p[:,0]));d['depth']=float(np.ptp(p[:,1]));assert d['depth']>=d['width'];d['inferred_front_faces']=len(faces)-original;d['front_transition']={'source_sha256':sha(source/'geometry.json'),'method':'Whole uncut leaves on inferred off-map branch clumps; stochastic sparse transition near native boundary. No complement-of-source background mask.','clusters':kept,'rejected_neighbor_boundary_faces':rejected_boundary,'retained_all_back_faces':original,'inside_map_front_claim':'Only limited uncut-leaf overhang; all known source/wood/neighbor firsthits must be tested. No new source ownership.'};d['status']='PRIVATE_NATIVE_TRANSITION_GEOMETRY_NOT_REVIEWED';(O/'geometry.json').write_text(json.dumps(d,separators=(',',':'))+'\n');shutil.copyfile(source/'native-donor-atlas.png',O/'native-donor-atlas.png')
 for label,name in [('native','tree01-native-cells-cpu-v5'),('static','tree01-static-cells-cpu-v3')]:
  old=B/name/'geometry.json';cell=json.loads(old.read_text());assert cell['inferred_crown_sha256']==sha(source/'geometry.json');cell['inferred_crown_sha256']=sha(O/'geometry.json');cell['identical_cells_input_sha256']=sha(old);cell['rebind_reason']='Same inferred support centers/radii; native cell vertices, UVs and exact source bytes unchanged.';(O/f'cells-{label}.json').write_text(json.dumps(cell,separators=(',',':'))+'\n')
 print({'back_faces':original,'front_faces':d['inferred_front_faces'],'width':d['width'],'depth':d['depth'],'outside_clusters':len(kept)})
if __name__=='__main__':main()

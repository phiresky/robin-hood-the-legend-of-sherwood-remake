"""Prepare inferred branch clumps using only the tree's own native leaf donors."""
import json,math,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'tree01-canopy-volume-cpu-v3';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=np.array([0.,-C,S])
def main():
 packet_path=B/'tree01-canopy-donors-v1/packet.json';packet=json.loads(packet_path.read_text());O.mkdir(exist_ok=False);atlas=Image.new('RGBA',(64,48));donors=[]
 for i,row in enumerate(packet['donors']):
  p=Path(row['path']);assert hashlib.sha256(p.read_bytes()).hexdigest()==row['sha256'];tile=Image.open(p).convert('RGBA');atlas.paste(tile,(i%4*16,i//4*16));donors.append({'box':[i%4*16,i//4*16,i%4*16+16,i//4*16+16],'native_box':row['native_box']})
 atlas.save(O/'native-donor-atlas.png')
 # Native supports correspond to the four upper woody tracks. Depth and the
 # portions beyond the map are explicit inferences, not measured source depth.
 supports=[{'native':[45,30],'depth_offset':-25.,'radii':[55.,85.,75.]},{'native':[82,-12],'depth_offset':22.,'radii':[52.,81.,64.]},{'native':[106,24],'depth_offset':-32.,'radii':[47.,85.,60.]},{'native':[153,14],'depth_offset':15.,'radii':[48.,77.,57.]},{'native':[108,116],'depth_offset':-10.,'radii':[45.,65.,47.]},{'native':[151,99],'depth_offset':12.,'radii':[42.,60.,52.]},{'native':[38,90],'depth_offset':-15.,'radii':[44.,60.,45.]}]
 rng=np.random.default_rng(31012026);vertices=[];faces=[];uvs=[];cluster_rows=[]
 for owner,support in enumerate(supports):
  x,y=support['native'];center=np.array([x,-303/S,(303-y)/C])+RAY*support['depth_offset'];radii=np.array(support['radii']);support['world_center']=center.tolist()
  for cluster in range(240 if owner<4 else 160):
   unit=rng.normal(size=3);unit/=np.linalg.norm(unit);unit*=rng.random()**(1/3);theta=math.atan2(unit[1],unit[0]);irregular=1+.13*math.sin(theta*5+owner)+.08*math.cos(theta*9-owner);position=center+unit*radii*irregular
   axis=rng.normal(size=3);axis/=np.linalg.norm(axis);other=np.cross(axis,[0.,0.,1.]if abs(axis[2])<.9 else[1.,0.,0.]);other/=np.linalg.norm(other);third=np.cross(axis,other);size=rng.uniform(4.5,8.5);donor=int(rng.integers(len(donors)));ax,ay,bx,by=donors[donor]['box'];coords=[[ax/64,1-ay/48],[bx/64,1-ay/48],[bx/64,1-by/48],[ax/64,1-by/48]]
   for u,v in [(axis,other),(axis,third),(other,third)]:
    points=[position+(u*su+v*sv)*size for su,sv in[(-1,-1),(1,-1),(1,1),(-1,1)]]
    # Inferred backs cannot become a second observed front owner. The eventual
    # material must be one-sided and saved/browser source-ray checks mandatory.
    if np.dot(np.cross(points[1]-points[0],points[2]-points[0]),RAY)>0:points=points[::-1]
    start=len(vertices);vertices.extend([p.tolist()for p in points]);uvs.extend(coords);faces.extend([[start,start+1,start+2],[start,start+2,start+3]])
   cluster_rows.append({'support':owner,'center':position.tolist(),'donor':donor,'size':size})
 points=np.array(vertices);width=float(np.ptp(points[:,0]));depth=float(np.ptp(points[:,1]));assert depth>=width,(width,depth);normals=np.cross(points[np.array(faces)[:,1]]-points[np.array(faces)[:,0]],points[np.array(faces)[:,2]]-points[np.array(faces)[:,0]]);assert np.max(normals@RAY)<=1e-8
 projected=-points[:,1]*S-points[:,2]*C;assert projected.min()<0;report={'status':'PRIVATE_INFERRED_CROWN_CPU_NOT_REVIEWED','vertices':vertices,'faces':faces,'uvs':uvs,'supports':supports,'clusters':cluster_rows,'width':width,'depth':depth,'projected_y_range':[float(projected.min()),float(projected.max())],'native_donor_packet_sha256':hashlib.sha256(packet_path.read_bytes()).hexdigest(),'atlas_sha256':hashlib.sha256((O/'native-donor-atlas.png').read_bytes()).hexdigest(),'observed_faces':0,'required_material':'One-sided inferred leaf backs, native alpha unchanged; no projected-source interior stripes.','required_review':['Saved model native/actual8/solid8 and full wood-ground context.','256/512 transparent-bounce convergence before modifying dense foliage.','Observed shared Arbre08 must occur exactly once, with native frame timing.','Native static crown/background ownership must be explicitly separated before final integration.'],'limits':['CPU envelope and leaves only; no accepted geometry, texture or ownership.','All unseen depth and off-map crown portions inferred.']};(O/'geometry.json').write_text(json.dumps(report,separators=(',',':'))+'\n');print({'clusters':len(cluster_rows),'faces':len(faces),'width':width,'depth':depth,'projected_y_range':report['projected_y_range']})
if __name__=='__main__':main()

"""Embed exact native fragment UVs on small inferred front cells for private review."""
import json,math,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'tree01-native-cells-cpu-v5';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=np.array([0.,-C,S]);SOURCE=B/'tree01-native-fragment-v1';FRAME='000.png';UNION='alpha-union.png';SOURCE_KIND='dynamic-frame0-provenance'
def main():
 source=SOURCE;scope=json.loads((source/'scope.json').read_text());crown_path=B/'tree01-canopy-volume-cpu-v6/geometry.json';crown=json.loads(crown_path.read_text());centers=np.array([row['world_center']for row in crown['supports']]);radii=np.array([row['radii']for row in crown['supports']]);union=np.array(Image.open(source/UNION))>0;vertices=[];uvs=[];faces=[];fallbacks=[]
 def point(x,y):
  origin=np.array([x,-303/S,(303-y)/C]);rel=origin-centers;a=np.sum((RAY/radii)**2,axis=1);b=2*np.sum(rel*RAY/radii**2,axis=1);c=np.sum((rel/radii)**2,axis=1)-1;disc=b*b-4*a*c;good=disc>=0
  if good.any():t=float(np.max(np.where(good,(-b+np.sqrt(np.maximum(disc,0)))/(2*a),-np.inf)))
  else:
   near=int(np.argmin(c-b*b/(4*a)));t=float(-b[near]/(2*a[near]));fallbacks.append([x,y])
  return origin+RAY*(t+2.)
 for top in range(0,175,6):
  for left in range(0,175,6):
   right=min(left+6,175);bottom=min(top+6,175)
   if not union[top:bottom,left:right].any():continue
   start=len(vertices)
   for x,y in [(left,top),(right,top),(right,bottom),(left,bottom)]:vertices.append(point(x,y).tolist());uvs.append([x/175,1-y/175])
   faces.extend([[start+2,start+1,start],[start+3,start+2,start]])
 points=np.array(vertices);uv=np.array(uvs);projected=np.column_stack((points[:,0],-points[:,1]*S-points[:,2]*C));expected=np.column_stack((uv[:,0]*175,(1-uv[:,1])*175));error=float(np.max(np.abs(projected-expected)));assert error<1e-7;normal=np.cross(points[np.array(faces)[:,1]]-points[np.array(faces)[:,0]],points[np.array(faces)[:,2]]-points[np.array(faces)[:,0]]);assert np.min(normal@RAY)>0
 O.mkdir(exist_ok=False);out={'status':'PRIVATE_NATIVE_FRAGMENT_EMBEDDING_NOT_OWNERSHIP_APPROVAL','source_role':SOURCE_KIND,'vertices':vertices,'faces':faces,'uvs':uvs,'source_frame':str(source/FRAME),'source_frame_sha256':hashlib.sha256((source/FRAME).read_bytes()).hexdigest(),'source_scope_sha256':hashlib.sha256((source/'scope.json').read_bytes()).hexdigest(),'inferred_crown_sha256':hashlib.sha256(crown_path.read_bytes()).hexdigest(),'source_uv_projection_error':error,'native_frame0_pixels':int((np.array(Image.open(source/FRAME))[:,:,3]>0).sum()),'native_union_pixels':int(union.sum()),'corner_nearest_branch_fallbacks':fallbacks,'limits':['Actual first-hit RGBA and wood/neighbor occlusion still require saved model checks.','Frame0 comparison geometry only; do not duplicate observed cells beside full native shared animation.','All3D placement depth is inferred;2D source RGBA and UVs are exact.']};(O/'geometry.json').write_text(json.dumps(out,separators=(',',':'))+'\n');print({'faces':len(faces),'native_uv_error':error,'fallback_corners':len(fallbacks)})
if __name__=='__main__':main()

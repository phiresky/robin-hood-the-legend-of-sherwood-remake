"""Join source-supported left stem sections through the leaf-covered interval."""
import json,hashlib,copy,argparse
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';I=B/'tree01-cpu-wood-v1/geometry.json';O=B/'tree01-cpu-wood-v2'
def main():
 global O
 ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,default=O);ap.add_argument('--smooth',action='store_true');args=ap.parse_args();O=args.output.resolve()
 assert not O.exists();O.mkdir();data=json.loads(I.read_text());by_id={p['source_obstacle_context']:p for p in data['parts']};upper,lower=by_id[3],by_id[2];n=upper['vertices_per_ring'];assert n==lower['vertices_per_ring'];vertices=copy.deepcopy(upper['vertices'][:upper['ring_count']*n]+lower['vertices'][:lower['ring_count']*n]);rings=upper['ring_count']+lower['ring_count'];faces=[];upper_kept=upper['ring_count'];bridge=0
 if args.smooth:
  ur=[np.array(upper['vertices'][i*n:(i+1)*n])for i in range(upper['ring_count']-1)];lr=[np.array(lower['vertices'][i*n:(i+1)*n])for i in range(1,lower['ring_count'])];a=ur[-1].mean(0);b=lr[0].mean(0);dz=b[2]-a[2];ta=a-ur[-2].mean(0);tb=lr[1].mean(0)-b;ta*=dz/ta[2];tb*=dz/tb[2];rad_a=(ur[-1][:,0].max()-ur[-1][:,0].min())/2;rad_b=(lr[0][:,0].max()-lr[0][:,0].min())/2;middle=[]
  for i in range(1,8):
   t=i/8;center=(2*t**3-3*t*t+1)*a+(t**3-2*t*t+t)*ta+(-2*t**3+3*t*t)*b+(t**3-t*t)*tb;radius=rad_a+(rad_b-rad_a)*t;middle.append(np.array([center+np.array([np.cos(2*np.pi*j/n)*radius,np.sin(2*np.pi*j/n)*radius,0])for j in range(n)]))
  vertices=np.concatenate(ur+middle+lr).tolist();rings=len(ur)+len(middle)+len(lr);upper_kept=len(ur);bridge=len(middle)
  assert np.array_equal(np.array(vertices[:len(ur)*n]),np.concatenate(ur)) and np.array_equal(np.array(vertices[-len(lr)*n:]),np.concatenate(lr))

 for row in range(rings-1):
  for i in range(n):
   a=row*n+i;b=row*n+(i+1)%n;c=b+n;d=a+n;faces += [[a,b,c],[a,c,d]]
 for row,reverse in [(0,True),(rings-1,False)]:
  center=len(vertices);vertices.append(np.mean(vertices[row*n:(row+1)*n],axis=0).tolist())
  for i in range(n):
   f=[center,row*n+i,row*n+(i+1)%n];faces.append(f[::-1]if reverse else f)
 volume=sum(float(np.dot(vertices[f[0]],np.cross(vertices[f[1]],vertices[f[2]])))/6 for f in faces)
 if volume<0:faces=[f[::-1]for f in faces];volume=-volume
 directed=Counter((a,b)for f in faces for a,b in zip(f,f[1:]+f[:1]));assert all(directed[(b,a)]==1 and count==1 for (a,b),count in directed.items());edges={tuple(sorted(e))for e in directed};assert len(vertices)-len(edges)+len(faces)==2 and volume>0
 joined={'source_obstacle_context':3,'source_contexts':[2,3],'vertices':vertices,'faces':faces,'volume':volume,'euler':2,'ring_count':rings,'vertices_per_ring':n,'observed_section_indices':upper['observed_section_indices']+[i+upper_kept+bridge-(1 if args.smooth else 0)for i in lower['observed_section_indices']],'hidden_extension_sections':[0,rings-1],'hidden_connection_ring_pair':[upper_kept-1,upper_kept+bridge],'width_depth_ratio':1.0,'source_ownership_assigned':False}
 data['parts']=[joined,by_id[4],by_id[5]];data['status']='HOLD_PRIVATE_JOINED_WOOD_NOT_VISUALLY_REVIEWED';data['parent_geometry']={'path':str(I),'sha256':hashlib.sha256(I.read_bytes()).hexdigest()};data['join_evidence']='tree01-left-join-proposal-v1/proposal.json';data['preservation']={'all_observed_ring_vertices_exact':True,'inferred_connection_smoothed':args.smooth,'old_buried_end_caps_removed':2,'gameplay_identities_unchanged':[2,3,4,5],'source_ownership_assigned':False};data['constraints']=[s for s in data['constraints']if not s.startswith('Four source-supported parts')]+['Upper3 and lower2 now share a continuous inferred wood surface through the native leaf-covered gap.','No new branches, root topology, canopy memberships or source texture pixels inferred.'];(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print({'parts':len(data['parts']),'joined_euler':2,'observed_ring_vertices_exact':True,'smooth_hidden_connection':args.smooth})
if __name__=='__main__':main()

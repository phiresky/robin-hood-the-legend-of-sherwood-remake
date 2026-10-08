"""Measure root volume contact with actual receiver surfaces along native rays."""
import argparse,json,numpy as np
from restart2_tree08_root_embedding_cpu import R,P,mesh,triangles,heights,sha
parser=argparse.ArgumentParser();parser.add_argument('--candidate',default='tree08-root-ray-cpu-v4');args=parser.parse_args();packet=R/args.candidate;assert packet.resolve().parent==R.resolve();out=packet/'receiver-intervals.json';assert not out.exists();s,c=np.sin(np.radians(35)),np.cos(np.radians(35))
def project(v):return np.c_[v[:,0],-v[:,1]*s-v[:,2]*c,-v[:,1]*c+v[:,2]*s]
m=np.load(packet/'candidate.npz');wood=triangles(project(m['vertices']),m['faces']);receivers=[]
for b in json.loads((P/'current-contact/receipt.json').read_text())['bindings']:
 v,f=mesh(b['asset']);v+=b['translation'];receivers.append((b['asset']['id'],triangles(project(v),f)))
rows=[]
for sample in json.loads((P/'current-contact/root-route-visibility.json').read_text())['samples']:
 x,y=sample['native'];q=[x+.5,y+.5];z=heights(wood,q);support=[(name,heights(t,q)) for name,t in receivers];support=[(name,float(h.max())) for name,h in support if len(h)];assert len(z) and support;name,depth=max(support,key=lambda x:x[1]);intervals=list(zip(z[::2],z[1::2]));inside=any(a-.0002<=depth<=b+.0002 for a,b in intervals);front=intervals[-1] if len(z)%2==0 else None;rows.append({'native':[x,y],'route':sample['route'],'receiver':name,'receiver_depth':depth,'wood_intervals':[[float(a),float(b)] for a,b in intervals],'receiver_surface_inside_wood':inside,'front_interval_back_clearance':None if front is None else float(front[0]-depth),'front_interval_receiver_contact':bool(front is not None and front[0]-.0002<=depth<=front[1]+.0002),'status':'AMBIGUOUS' if len(z)%2 else 'RECEIVER_INTERSECTS_WOOD' if inside else 'NO_RECEIVER_INTERSECTION'})
report={'candidate_sha256':sha(packet/'candidate.npz'),'counts':{k:sum(x['status']==k for x in rows) for k in sorted({x['status'] for x in rows})},'samples':rows,'scope':'Actual bound receiver surface lies inside closed wood ray intervals. Includes contact with terrace sides; unlike vertical support columns, does not misclassify a root attached to a bank face as necessarily floating. Contact samples are not a complete underside or anatomy proof.'};out.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['counts']))

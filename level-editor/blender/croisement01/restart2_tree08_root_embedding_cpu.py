"""Check vertical wood intervals against unchanged bound receiver surfaces."""
import hashlib,json,struct
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
P=R/'tree08-wood-prototype-v14-root-ray'
O=R/'tree08-root-embedding-cpu-v1';O.mkdir(exist_ok=False)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def mesh(asset):
 p=ROOT/'level-editor/library'/asset['model'];assert sha(p)==asset['model_sha256']
 b=p.read_bytes();n=struct.unpack_from('<I',b,12)[0];g=json.loads(b[20:20+n]);buf=[(p.parent/x['uri']).read_bytes() if 'uri' in x else b[28+n:] for x in g['buffers']]
 def a(i):
  q=g['accessors'][i];v=g['bufferViews'][q['bufferView']];d=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2'}[q['componentType']]);w={'VEC3':3,'SCALAR':1}[q['type']]
  return np.ndarray((q['count'],w),dtype=d,buffer=buf[v['buffer']],offset=v.get('byteOffset',0)+q.get('byteOffset',0),strides=(v.get('byteStride',w*d.itemsize),d.itemsize)).copy()
 assert len(g['meshes'])==1 and len(g['meshes'][0]['primitives'])==1
 q=g['meshes'][0]['primitives'][0];return a(q['attributes']['POSITION']).astype(float),a(q['indices']).reshape(-1,3).astype(int)
def triangles(v,f):
 t=v[f];u=t[:,1,:2]-t[:,0,:2];w=t[:,2,:2]-t[:,0,:2];det=u[:,0]*w[:,1]-u[:,1]*w[:,0];return t,u,w,det
def heights(packet,xy):
 t,u,w,det=packet;d=np.array(xy)-t[:,0,:2];valid=abs(det)>1e-9;safe=np.where(valid,det,1);a=(d[:,0]*w[:,1]-d[:,1]*w[:,0])/safe;b=(u[:,0]*d[:,1]-u[:,1]*d[:,0])/safe
 ok=valid&(a>=-1e-8)&(b>=-1e-8)&(a+b<=1+1e-8);z=t[ok,0,2]+a[ok]*(t[ok,1,2]-t[ok,0,2])+b[ok]*(t[ok,2,2]-t[ok,0,2]);z=np.sort(z)
 return z[np.r_[True,np.diff(z)>1e-5]] if len(z) else z
m=np.load(R/'tree08-root-ray-cpu-v4/candidate.npz');wood=triangles(m['vertices'],m['faces']);receipt=json.loads((P/'current-contact/receipt.json').read_text());receivers=[]
for binding in receipt['bindings']:
 v,f=mesh(binding['asset']);v+=binding['translation'];receivers.append((binding['asset']['id'],triangles(v,f)))
route=json.loads((P/'current-contact/root-route-visibility.json').read_text());rows=[]
for sample in route['samples']:
 assert sample['wood_hit'] is not None
 hit=sample['wood_hit'];z=heights(wood,hit[:2]);support=[(name,heights(t,hit[:2])) for name,t in receivers];support=[(name,float(h.max())) for name,h in support if len(h)];assert support
 # Find the closed vertical interval containing the observed source-facing hit.
 intervals=list(zip(z[::2],z[1::2]));matches=[(lo,hi) for lo,hi in intervals if lo-0.01<=hit[2]<=hi+0.01]
 row={'native':sample['native'],'route':sample['route'],'wood_hit':hit,'vertical_intersection_count':len(z),'receiver_top':max(support,key=lambda x:x[1]),'matched_intervals':[[float(a),float(b)] for a,b in matches]}
 if len(z)%2 or len(matches)!=1:row['status']='AMBIGUOUS_INTERVAL'
 else:
  lo,hi=matches[0];top=row['receiver_top'][1];row.update(bottom_minus_receiver=float(lo-top),top_minus_receiver=float(hi-top),status='FLOATING' if lo>top+0.05 else ('BURIED' if hi<top-0.05 else 'INTERSECTS_RECEIVER_HEIGHT'))
 rows.append(row)
counts={k:sum(x['status']==k for x in rows) for k in sorted({x['status'] for x in rows})}
out={'model_sha256':receipt['model_sha256'],'candidate_sha256':sha(R/'tree08-root-ray-cpu-v4/candidate.npz'),'counts':counts,'samples':rows,'scope':'Vertical interval test at traced source-facing root samples. Height intersection is not full receiver-volume intersection or soil anatomy approval. No geometry changed.'}
(O/'report.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(counts));print('floating examples',json.dumps([x for x in rows if x['status']=='FLOATING'][:3]))

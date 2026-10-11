"""Locally taper proposed motion to retain saved triangle orientation and area."""
import argparse,json,struct,hashlib,copy
from pathlib import Path
import numpy as np
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--export',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);report=json.loads(a.export.read_text());out=copy.deepcopy(report)
 for asset in out['assets']:
  path=Path(asset['model']['path']);assert sha(path)==asset['model']['sha256'];raw=bytearray(path.read_bytes());n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);start=28+n
  def read(i):
   x=doc['accessors'][i];v=doc['bufferViews'][x['bufferView']];dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[x['componentType']];width={'SCALAR':1,'VEC3':3}[x['type']];item=np.dtype(dtype).itemsize
   return np.ndarray((x['count'],width),dtype=dtype,buffer=raw,offset=start+v.get('byteOffset',0)+x.get('byteOffset',0),strides=(v.get('byteStride',item*width),item))
  prims=doc['meshes'][doc['nodes'][asset['crown_node']]['mesh']]['primitives'];positions=[read(q['attributes']['POSITION']).copy() for q in prims];combined=np.concatenate(positions);unique,inverse=np.unique(combined,axis=0,return_inverse=True);offset=0;parts=[];triangles=[]
  for prim,pos in zip(prims,positions):
   ids=inverse[offset:offset+len(pos)];offset+=len(pos);ix=read(prim['indices']).reshape(-1,3).copy();parts.append((prim,pos,ids));triangles.append(ids[ix])
  triangles=np.concatenate(triangles);basis=unique[triangles];norm=np.cross(basis[:,1]-basis[:,0],basis[:,2]-basis[:,0]);area=np.linalg.norm(norm,axis=1);valid=area>1e-8;factors=np.ones(len(unique));deltas=np.zeros((13,len(unique),3));seen=np.zeros(len(unique),dtype=bool)
  for prim,pos,ids in parts:
   d=np.array([read(t['POSITION']).copy() for t in prim['targets']]);known=seen[ids]
   if known.any():assert np.max(np.abs(d[:,known]-deltas[:,ids[known]]))<1e-5
   deltas[:,ids]=d;seen[ids]=True
  edges=np.unique(np.sort(np.concatenate([triangles[:,[0,1]],triangles[:,[1,2]],triangles[:,[2,0]]]),axis=1),axis=0)
  for iteration in range(64):
   bad=np.zeros(len(triangles),dtype=bool)
   for d in deltas:
    moved=(unique+d*factors[:,None]).astype(np.float32)[triangles];nn=np.cross(moved[:,1]-moved[:,0],moved[:,2]-moved[:,0]);bad|=valid&((np.sum(nn*norm,axis=1)<=.05*area**2)|(np.linalg.norm(nn,axis=1)<.05*area))
   if not bad.any():break
   factors[np.unique(triangles[bad])]*=.5
   # Bound neighboring attenuation changes; duplicate primitive vertices share one factor.
   for _ in range(10):
    old=factors.copy();np.minimum.at(factors,edges[:,0],old[edges[:,1]]+.1);np.minimum.at(factors,edges[:,1],old[edges[:,0]]+.1)
    if np.array_equal(old,factors):break
  else:raise AssertionError('Cannot preserve triangle orientation with local taper')
  for prim,pos,ids in parts:
   for phase,t in enumerate(prim['targets']):
    target=read(t['POSITION']);target[:]=deltas[phase,ids]*factors[ids,None];accessor=doc['accessors'][t['POSITION']];accessor['min']=target.min(0).tolist();accessor['max']=target.max(0).tolist()
  encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);binary=bytes(raw[start:]);output=a.output/path.name;output.write_bytes(struct.pack('<III',0x46546c67,2,28+len(encoded)+len(binary))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+struct.pack('<II',len(binary),0x004e4942)+binary)
  asset['untapered_model']=asset['model'];asset['model']=dict(path=str(output),sha256=sha(output));asset['motion_taper']=dict(iterations=iteration,minimum=float(factors.min()),affected_vertices=int((factors<1).sum()),total_unique_vertices=len(unique),mean=float(factors.mean()),scope='One common factor per coincident crown vertex across all13phases; motion only, basis/materials/UVs unchanged; inferred local attenuation to preserve triangle orientation.')
  print(asset['asset_id'],asset['motion_taper'],flush=True)
 out['untapered_export']=dict(path=str(a.export),sha256=sha(a.export));out['taper_recipe_sha256']=sha(Path(__file__));out['status']='PRIVATE_TAPERED_MOTION_REQUIRES_NEW_IMPORT_AND_REVIEW';(a.output/'export.json').write_text(json.dumps(out,indent=2)+'\n')
if __name__=='__main__':main()

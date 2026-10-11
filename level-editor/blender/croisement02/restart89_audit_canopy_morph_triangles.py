"""Check saved canopy motion for degenerate or reversed triangles and edge strain."""
import argparse,json,struct,hashlib
from pathlib import Path
import numpy as np
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--export',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False)
 report=json.loads(a.export.read_text());rows=[]
 for asset in report['assets']:
  path=Path(asset['model']['path']);assert sha(path)==asset['model']['sha256'];raw=path.read_bytes();n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);data=raw[28+n:]
  def read(i):
   x=doc['accessors'][i];v=doc['bufferViews'][x['bufferView']];assert v.get('buffer',0)==0 and 'sparse' not in x
   dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[x['componentType']];width={'SCALAR':1,'VEC3':3}[x['type']];item=np.dtype(dtype).itemsize
   return np.ndarray((x['count'],width),dtype=dtype,buffer=data,offset=v.get('byteOffset',0)+x.get('byteOffset',0),strides=(v.get('byteStride',item*width),item)).copy()
  phases=[];mesh=doc['meshes'][doc['nodes'][asset['crown_node']]['mesh']]
  for number,prim in enumerate(mesh['primitives']):
   assert prim.get('mode',4)==4;pos=read(prim['attributes']['POSITION']);indices=read(prim['indices']).reshape(-1,3);base=pos[indices];base_normal=np.cross(base[:,1]-base[:,0],base[:,2]-base[:,0]);base_area=np.linalg.norm(base_normal,axis=1);valid=base_area>1e-8
   original_edges=np.linalg.norm(base[:,[1,2,0]]-base,axis=2);edge_valid=original_edges>1e-6
   for phase,target in enumerate(prim['targets'],1):
    moved=(pos+read(target['POSITION']))[indices];norm=np.cross(moved[:,1]-moved[:,0],moved[:,2]-moved[:,0]);area=np.linalg.norm(norm,axis=1);dots=np.sum(norm*base_normal,axis=1);flips=np.flatnonzero(valid&(dots<=0));collapse=np.flatnonzero(valid&(area<base_area*.01));stretch=np.linalg.norm(moved[:,[1,2,0]]-moved,axis=2)[edge_valid]/original_edges[edge_valid]
    phases.append(dict(primitive=number,phase=phase,triangles=len(indices),original_degenerates=int((~valid).sum()),reversed_triangle_count=len(flips),reversed_first=flips[:20].tolist(),near_collapsed_triangle_count=len(collapse),edge_ratio_min=float(stretch.min()),edge_ratio_max=float(stretch.max())))
  rows.append(dict(asset_id=asset['asset_id'],model_sha256=sha(path),phases=phases))
 issues=sum(r['reversed_triangle_count']+r['near_collapsed_triangle_count'] for x in rows for r in x['phases'])
 result=dict(status='MOTION_TRIANGLE_REFINEMENT_REQUIRED' if issues else 'PASS_SAVED_MORPH_TRIANGLE_ORIENTATION_AND_AREA',export_sha256=sha(a.export),recipe_sha256=sha(Path(__file__)),issues=issues,assets=rows,limits=['Does not establish self-intersection freedom, neighbor contact, source silhouette parity, or approval.','Triangle orientation measured relative to its own static basis; original degenerate faces are disclosed separately.'])
 (a.output/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(result['status'],issues)
if __name__=='__main__':main()

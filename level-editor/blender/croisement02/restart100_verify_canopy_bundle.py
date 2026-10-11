"""Independently verify bundled hierarchy, surface bytes, and synchronized morph routing."""
import argparse,json,struct,hashlib
from pathlib import Path
import numpy as np
from scipy.spatial.transform import Rotation
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def load(p):
 b=p.read_bytes();n=struct.unpack_from('<I',b,12)[0];return json.loads(b[20:20+n]),b[28+n:]
def main():
 p=argparse.ArgumentParser();p.add_argument('--bundle',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);report=json.loads(a.bundle.read_text());model=Path(report['model']['path']);assert sha(model)==report['model']['sha256'];doc,data=load(model);checks=[]
 def accessor(d,b,i):
  x=d['accessors'][i];v=d['bufferViews'][x['bufferView']];width={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4,'MAT4':16}[x['type']];dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1',5122:'<i2',5120:'i1'}[x['componentType']];size=np.dtype(dtype).itemsize;return np.ndarray((x['count'],width),dtype=dtype,buffer=b,offset=v.get('byteOffset',0)+x.get('byteOffset',0),strides=(v.get('byteStride',size*width),size)).copy()
 def world(d):
  result={}
  def visit(i,parent):
   n=d['nodes'][i]
   if 'matrix' in n:m=np.array(n['matrix']).reshape(4,4).T
   else:m=np.eye(4);m[:3,:3]=Rotation.from_quat(n.get('rotation',[0,0,0,1])).as_matrix()@np.diag(n.get('scale',[1,1,1]));m[:3,3]=n.get('translation',[0,0,0])
   result[i]=parent@m
   for child in n.get('children',[]):visit(child,result[i])
  for i in d['scenes'][d.get('scene',0)]['nodes']:visit(i,np.eye(4))
  return result
 def material(d,b,index):
  m=json.loads(json.dumps(d['materials'][index]))
  def visit(value):
   if isinstance(value,dict):
    for key,item in value.items():
     if key.endswith('Texture') and isinstance(item,dict) and 'index' in item:
      t=d['textures'][item.pop('index')];im=d['images'][t['source']];v=d['bufferViews'][im['bufferView']];start=v.get('byteOffset',0);item['resolved_image_sha256']=hashlib.sha256(b[start:start+v['byteLength']]).hexdigest();item['resolved_sampler']=d['samplers'][t['sampler']] if 'sampler' in t else {}
     else:visit(item)
   elif isinstance(value,list):
    for item in value:visit(item)
  visit(m);return m
 actual_world=world(doc);assert len(doc['animations'])==1;channels=doc['animations'][0]['channels'];samplers=doc['animations'][0]['samplers']
 for asset in report['assets']:
  source=Path(asset['source_model']['path']);assert sha(source)==asset['source_model']['sha256'];src,sb=load(source);sw=world(src);offset=asset['node_range'][0];t=asset['placement']['transform'];placement=np.eye(4);placement[:3,3]=[t['dx'],t['dz'],t['dy']/np.sin(np.radians(35))];primitives=0;vertices=0
  for i,expected in sw.items():
   assert np.allclose(actual_world[offset+i],placement@expected,rtol=0,atol=1e-9)
   sn=src['nodes'][i];dn=doc['nodes'][offset+i]
   if 'mesh' not in sn:continue
   sm=src['meshes'][sn['mesh']];dm=doc['meshes'][dn['mesh']];assert sm.get('weights')==dm.get('weights') and len(sm['primitives'])==len(dm['primitives'])
   for sp,dp in zip(sm['primitives'],dm['primitives']):
    assert set(sp['attributes'])==set(dp['attributes']);assert sp.get('mode',4)==dp.get('mode',4);assert material(src,sb,sp['material'])==material(doc,data,dp['material'])
    for key in sp['attributes']:assert np.array_equal(accessor(src,sb,sp['attributes'][key]),accessor(doc,data,dp['attributes'][key]))
    assert np.array_equal(accessor(src,sb,sp['indices']),accessor(doc,data,dp['indices']));assert len(sp.get('targets',[]))==len(dp.get('targets',[]))
    for st,dt in zip(sp.get('targets',[]),dp.get('targets',[])):
     assert st.keys()==dt.keys()
     for key in st:assert np.array_equal(accessor(src,sb,st[key]),accessor(doc,data,dt[key]))
    primitives+=1;vertices+=src['accessors'][sp['attributes']['POSITION']]['count']
  for channel in src['animations'][0]['channels']:
   found=[x for x in channels if x['target']==dict(node=channel['target']['node']+offset,path=channel['target']['path'])];assert len(found)==1;ss=src['animations'][0]['samplers'][channel['sampler']];ds=samplers[found[0]['sampler']];assert ss['interpolation']==ds['interpolation']=='STEP'
   for key in ['input','output']:assert np.array_equal(accessor(src,sb,ss[key]),accessor(doc,data,ds[key]))
  checks.append(dict(asset_id=asset['asset_id'],hierarchy_nodes=len(sw),primitives=primitives,vertices=vertices,world_placement_and_all_surface_bytes_exact=True,morph_route_exact=True))
 (a.output/'report.json').write_text(json.dumps(dict(status='PASS_COMBINED_HIERARCHY_SURFACE_AND_MORPH_ROUTING',bundle_report_sha256=sha(a.bundle),model_sha256=sha(model),assets=checks,limits=['Core graph readback only; actual importer/browser and combined appearance remain pending.','New inferred motion unapproved.']),indent=2)+'\n');print(len(checks),'bundled crowns verified')
if __name__=='__main__':main()

"""Conservatively compare full cubic butterfly paths against independent canopy phase bounds."""
import argparse,json,struct,hashlib
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/active/croisement02'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def bounds(path):
 raw=path.read_bytes();length=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+length]);blob=raw[28+length:]
 def acc(i):
  a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];assert a['componentType']==5126 and not a.get('sparse') and not v.get('byteStride');width={'SCALAR':1,'VEC3':3,'VEC4':4}[a['type']];return np.frombuffer(blob,dtype='<f4',count=a['count']*width,offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(a['count'],width).astype(float)
 assert len(doc['animations'])==1;animation=doc['animations'][0];assert sum(x['target']['path']=='translation' for x in animation['channels'])==1;assert all(x['target']['path'] in ('translation','rotation') for x in animation['channels']);channel=next(x for x in animation['channels'] if x['target']['path']=='translation');root=channel['target']['node'];sampler=animation['samplers'][channel['sampler']];assert sampler['interpolation']=='CUBICSPLINE';times=acc(sampler['input'])[:,0];values=acc(sampler['output']).reshape(-1,3,3);radii=[]
 def visit(i,offset):
  n=doc['nodes'][i];assert np.allclose(n.get('scale',[1,1,1]),[1,1,1]);assert 'matrix' not in n
  if i!=root:offset+=np.linalg.norm(n.get('translation',[0,0,0]))
  if 'mesh'in n:
   for p in doc['meshes'][n['mesh']]['primitives']:
    assert not p.get('targets');radii.append(float(np.linalg.norm(acc(p['attributes']['POSITION']),axis=1).max()+offset))
  for child in n.get('children',[]):visit(child,offset)
 visit(root,0);radius=max(radii)+.001;out=[]
 for i,dt in enumerate(np.diff(times)):
  controls=np.array([values[i,1],values[i,1]+values[i,2]*dt/3,values[i+1,1]-values[i+1,0]*dt/3,values[i+1,1]])
  out.append((controls.min(0)-radius,controls.max(0)+radius))
 return np.asarray(out),radius

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--model',type=Path,required=True);p.add_argument('--number',type=int,required=True);a=p.parse_args();out=a.output;out.mkdir(exist_ok=False);cp=B/'canopy-current-static-clearance-refined-oct12-v1/audit.json';canopies=json.loads(cp.read_text());models={a.number:a.model.parent};rows=[]
 for number,folder in models.items():
  path=a.model.resolve();sweeps,radius=bounds(path);pairs=[]
  for crown in canopies['crowns']:
   allbounds=np.array([x for phase in crown['phases'] for x in phase['primitive_exact_vertex_bounds']]);lo=allbounds[:,0].min(0);hi=allbounds[:,1].max(0);indices=np.flatnonzero(np.all(sweeps[:,1]>=lo,axis=1)&np.all(sweeps[:,0]<=hi,axis=1))
   if len(indices):pairs.append({'asset':crown['asset'],'group':crown['group'],'possible_translation_segments':len(indices),'first_segment':int(indices[0]),'last_segment':int(indices[-1])})
  rows.append({'butterfly':number,'model':str(path),'model_sha256':sha(path),'rigid_radius':radius,'translation_segments':len(sweeps),'possible_canopy_pairs':pairs})
 result={'status':'CONSERVATIVE_INDEPENDENT_CLOCK_BROADPHASE_ONLY','canopy_bound_source':str(cp),'canopy_bound_sha256':sha(cp),'recipe_sha256':sha(Path(__file__)),'rows':rows,'method':'Each exact glTF cubic translation segment lies within its Bernstein control-point hull. A sphere covers every rigid articulated vertex regardless of relative wing/body rotations. Crown bounds union all14 independent poses. Disjoint AABBs prove separation; overlaps do not prove collision.','limits':['Exact named model only; later model changes require a new audit.','Only seven uninstalled canopy groups in supplied audit; installed Tree42 has separate exact phase audits.','Possible pairs require exact phase-triangle swept tests or stronger separating bounds before whole-scene motion clearance.']};(out/'audit.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['butterfly'],len(r['possible_canopy_pairs']))for r in rows])
if __name__=='__main__':main()

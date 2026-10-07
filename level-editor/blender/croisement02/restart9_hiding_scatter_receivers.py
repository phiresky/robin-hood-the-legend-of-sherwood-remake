"""Pin terrain support for leaf-cover and scattered-leaf physical endpoints."""
from pathlib import Path
import sys,json,hashlib,collections
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=OUT/'restart9-hiding-scatter/terrain-receivers-v2';out.mkdir(parents=True,exist_ok=False)
 base=OUT/'restart7-source-patch-delivery/contracts-v1';manifest=json.loads((base/'manifest.json').read_text());resources={r['path']:Path(r['source'])for r in manifest['resources']};records=[]
 for r in manifest['records']:
  if not(r['profile'].endswith('hiding Pc')or r['profile'].endswith('piege01g')):continue
  c=json.loads((base/r['contract']).read_text());p=next(x for x in c['native']['patch_states']if x['id']==r['id']);frames=[p['initial'][0],p['transition'][-1]if p['integrate_in_background']else (p['final'][0] if p['final'] else None)];samples=set()
  for f in frames:
   if f is None:continue
   path=resources[f['path']];assert sha(path)==f['sha256'];a=np.asarray(Image.open(path).convert('RGBA'));yy,xx=np.where(a[:,:,3]>0)
   for y,x in zip(yy,xx):
    if x%3==0 and y%3==0:samples.add((float(p['display_position'][0]+f['offset'][0]+x)+.5,float(p['display_position'][1]+f['offset'][1]+y)+.5))
  records.append(dict(id=r['id'],profile=r['profile'],contract=str(base/r['contract']),contract_sha256=sha(base/r['contract']),display_position=p['display_position'],native_order_elevation=p['elevation'],initial=frames[0],applied=frames[1],samples=[dict(pixel=list(q),candidates=[])for q in sorted(samples)]))
 models=[('flat',OUT/'restart4-fence14-ground-candidate-v1/model.blend','4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8'),('bank',OUT/'restart4-bank103-source-overlay-v2/model.blend','c0b8458ed39b0412fae16a03384c5585c3271cc5eb48072f0354380ab8f0072e')];pins=[]
 for kind,path,digest in models:
  assert sha(path)==digest;bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and ((kind=='flat'and o.name=='Croisement02 Terrain')or(kind=='bank'and o.get('asset_group')=='croisement02-north-woodland-bank'))];assert objects;tree,owners,_=_tree(objects);pins.append(dict(path=str(path),sha256=digest,objects=[dict(name=o.name,matrix_world=[list(row)for row in o.matrix_world])for o in objects]))
  for record in records:
   for sample in record['samples']:
    x,y=sample['pixel'];point,normal,index,distance=tree.ray_cast(Vector((x,-y/SIN,0))+RAY*6000,-RAY)
    if point is not None:sample['candidates'].append(dict(receiver=owners[index].name,kind=kind,point=list(point),normal=list(normal),distance=float(distance)))
 for record in records:
  for sample in record['samples']:
   assert sample['candidates'],sample;sample['support']=min(sample['candidates'],key=lambda r:r['distance']);del sample['candidates']
  record['receiver_counts']=dict(collections.Counter(s['support']['receiver']for s in record['samples']));record['height_range']=[min(s['support']['point'][2]for s in record['samples']),max(s['support']['point'][2]for s in record['samples'])]
 report=dict(scope='Sampled terrain support only. Native sprite elevation is draw ordering, not physical height. Foreground wood/foliage occlusion and nonterrain props are not included; contact is not full-scene source-visibility proof.',models=pins,records=records,instance_count=len(records),sample_count=sum(len(r['samples'])for r in records))
 (out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([{k:r[k]for k in ['id','height_range','receiver_counts']}for r in records]),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()

"""Restore only live crown sampling in private deliveries, retaining binary payloads."""
from pathlib import Path
import copy,hashlib,json,struct,sys
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement/blender')]
from restart8_audit_live_crowns import material
from lossy_assets import read_glb
B=R/'level-editor/work/croisement02-refinement/restart8-five-bark-approved-export-v1'
def sha(b):return hashlib.sha256(b).hexdigest()
def unpack(raw):
 n,k=struct.unpack_from('<II',raw,12);assert k==0x4e4f534a
 return json.loads(raw[20:20+n]),raw[20+n:]
def main():
 audit_path=B/'live-crown-audit-v1.json';audit=json.loads(audit_path.read_text())
 for row in audit['trees']:
  n=row['tree'];base=B/f'tree-{n}-v1';out=base/'delivery-v4';out.mkdir(exist_ok=False);old=Path(row['baseline']);assert sha(old.read_bytes())==row['baseline_sha256'];od,ob,_=read_glb(old);on=next(x for x in od['nodes']if'Crown'in x.get('name',''));op=od['meshes'][on['mesh']]['primitives'];records=[]
  for file in ['model.glb','lossless.glb']:
   src=base/'delivery-v3'/file;raw=src.read_bytes();d,tail=unpack(raw);original=copy.deepcopy(d);node=next(x for x in d['nodes']if'Crown'in x.get('name',''));primitives=d['meshes'][node['mesh']]['primitives'];changed=set()
   def repair(oldmat,newmat):
    for key,value in oldmat.items():
     if key.endswith('Texture')and isinstance(value,dict)and'index'in value:
      oldtex=od['textures'][value['index']];sampler=od.get('samplers',[])[oldtex['sampler']]if'sampler'in oldtex else{}
      info=newmat[key];tex=copy.deepcopy(d['textures'][info['index']]);samplers=d.setdefault('samplers',[])
      if sampler not in samplers:samplers.append(copy.deepcopy(sampler))
      tex['sampler']=samplers.index(sampler)
      if tex not in d['textures']:d['textures'].append(tex)
      info['index']=d['textures'].index(tex)
     elif isinstance(value,dict):repair(value,newmat[key])
   for oldp,newp in zip(op,primitives):
    mi=newp['material'];before=copy.deepcopy(d['materials'][mi]);repair(od['materials'][oldp['material']],d['materials'][mi])
    if before!=d['materials'][mi]:changed.add(mi)
   assert changed
   for i,m in enumerate(original['materials']):
    if i not in changed:assert d['materials'][i]==m
   check=copy.deepcopy(d)
   for i in changed:check['materials'][i]=original['materials'][i]
   check['textures']=check['textures'][:len(original['textures'])];check['samplers']=check['samplers'][:len(original.get('samplers',[]))];assert check==original
   encoded=json.dumps(d,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);result=struct.pack('<4sII',b'glTF',2,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+tail;(out/file).write_bytes(result);assert unpack(result)[1]==tail
   nd,nb,_=read_glb(out/file)
   for oldp,newp in zip(op,primitives):assert material(old,od,ob,oldp['material'])==material(out/file,nd,nb,newp['material'])
   records.append(dict(file=file,source_sha256=sha(raw),output_sha256=sha(result),binary_chunks_exact=True,all_geometry_uv_images_exact=True,all_bark_materials_exact=True,all_other_materials_exact=True,only_crown_texture_sampler_references_changed=True,crown_materials_match_frozen_live_semantics=True,changed_materials=sorted(changed)))
  (out/'asset.json').write_bytes((base/'delivery-v3/asset.json').read_bytes());proof=dict(status='PASS live crown sampler restoration; fresh visual review pending',tree=n,audit_sha256=sha(audit_path.read_bytes()),baseline_model_sha256=row['baseline_sha256'],files=records,source_worker_unchanged=True,publication=False);(base/'live-crown-sampler-guard-v1.json').write_text(json.dumps(proof,indent=2)+'\n');print(n,records[-1]['output_sha256'],flush=True)
if __name__=='__main__':main()

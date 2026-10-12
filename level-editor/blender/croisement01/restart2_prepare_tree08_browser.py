"""Prepare CPU-only normal-editor Tree08 proof; browser owner runs it separately."""
import hashlib,json,os,shutil,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];R=ROOT/'level-editor/work/active/croisement01/restart2';os.chdir(ROOT)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def write(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
def rel(p):return str(p.relative_to(ROOT))
base=R/'tree02-browser-private-v2';out=R/'tree08-browser-private-v1';stage=R/'tree08-native33-integration-stage-v2';lib=out/'private-library';oldlib=R/'tree02-publication-plan-v1/browser-private-library-v2'
assert sha(ROOT/'level-editor/library/scenes/croisement01.rhlos-map.json')==read(stage/'stage-proof.json')['source_map_sha256']
assert read(R/'tree08-native33-export-v2/reviewed-export-equivalence.json')['status']=='PASS_REVIEWED_GEOMETRY_AND_APPEARANCE_PRESERVED'
out.mkdir();shutil.copytree(oldlib,lib,copy_function=os.link,symlinks=True)
# Unlink private hardlinks before replacing their contents.
scene=lib/'scenes/croisement01.rhlos-map.json';scene.unlink();shutil.copy2(stage/scene.name,scene)
asset='croisement01-tree-08';shutil.copytree(stage/'assets'/asset,lib/'3d-assets/croisement01'/asset,copy_function=os.link)
index=read(lib/'3d-assets/index.json');entry=read(stage/'assets/index.json')['assets'][0]
entry['model']='croisement01/'+entry['model'];entry['descriptor']='croisement01/'+entry['descriptor']
assert entry['id']==asset
assert sum(x['id']=='croisement01-group-033' for x in index['assets'])==1
index['assets']=[entry if x['id']=='croisement01-group-033' else x for x in index['assets']]
p=lib/'3d-assets/index.json';p.unlink();p.write_text(json.dumps(index,separators=(',',':'))+'\n')
for name in ['run.mjs','editor.tsx','observe-library.js','check.js']:
 s=(base/name).read_text().replace(rel(base),rel(out)).replace(rel(oldlib),rel(lib)).replace('tree02','tree08').replace('Tree02','Tree08').replace('croisement01-tree-02','croisement01-tree-08')
 if name=='run.mjs':s=s.replace('level-editor/work/active/croisement01/restart2/tree08-browser-private-v1/reload-check.js',rel(out/'reload-check.js'))
 if name=='observe-library.js':s=s.replace('(tree-02|group-005)','tree-08')
 if name=='check.js':
  s=s.replace("x.id==='tree08-approved'","x.id==='group-033'")
  start=s.index("check('Inferred crown carries scenery identity'");end=s.index(" for(const group of [tree,retained])",start)
  s=s[:start]+"check('Original native placement preserved',same(tree.transform,cfg.placement.transform),tree.transform);\n"+s[end:]
  s=s.replace('for(const group of [tree,retained])','for(const group of [tree])')
  s=s.replace('Private staged library: approved Tree08 lower/upper wood and inferred crown, with five retained group005 parts.','Private staged library: approved Tree08 wood replacing native obstacle033 visual with exact retained gameplay. Canopy excluded.')
 (out/name).write_text(s)
(out/'reload-check.js').write_text((R/'tree02-browser-private-v1/reload-check.js').read_text().replace('tree02','tree08'))
proof=read(stage/'stage-proof.json');m=read(scene);oldcfg=read(base/'check-config.json');pin=proof['new_source'];cfg=dict(proof={'assets':{asset:{k:pin[k] for k in ['model_sha256','descriptor_sha256']}}},tree_parts=['building-033'],expected_counts=oldcfg['expected_counts'],stage_map=m,placement=next(p for p in m['placements'] if p['id']=='group-033'))
write(out/'check-config.json',cfg)
authority=read(base/'runtime-authority.json');snapshot={}
for directory in authority['sourceDirectories']:
 for p in sorted((ROOT/directory).rglob('*')):
  if p.is_file() and not any(p.name.endswith('.test.'+ext) for ext in ['ts','mjs','js']):
   snapshot[rel(p)]=sha(p)
   if p.suffix in ['.ts','.tsx','.js','.jsx','.mjs','.cjs','.css','.json']:
    dest=out/'snapshot'/rel(p);dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,dest)
subset=sorted([x for x in index['assets'] if x.get('source_map','').lower()=='croisement01'],key=lambda x:x['id'])
# JS stable serialisation uses UTF-8, not ASCII escaping.
stable=json.dumps(subset,sort_keys=True,separators=(',',':'),ensure_ascii=False)
stable_hash=subprocess.run(['node','--input-type=module','-e',"import fs from 'node:fs';import crypto from 'node:crypto';process.stdout.write(crypto.createHash('sha256').update(JSON.stringify(JSON.parse(fs.readFileSync(0,'utf8')))).digest('hex'));"],input=stable,text=True,capture_output=True,check=True).stdout
authority['membership']=sorted(snapshot);authority['index']=dict(path=rel(lib/'3d-assets/index.json'),initial_sha256=sha(lib/'3d-assets/index.json'),c02_count=len(subset),c02_sha256=stable_hash)
write(out/'source-snapshot.json',snapshot);write(out/'runtime-authority.json',authority)
runtime=dict(snapshot)
for path in read(base/'runtime-inputs.json'):
 if path in read(base/'source-snapshot.json'):continue
 p=ROOT/path.replace(rel(oldlib),rel(lib));runtime[rel(p)]=sha(p)
for ref in m['assetSources']:
 for key in ['model','descriptor']:
  p=lib/ref[key];assert sha(p)==ref[key+'_sha256'];runtime[rel(p)]=sha(p)
runtime[rel(scene)]=sha(scene);write(out/'runtime-inputs.json',runtime)
inputs={rel(out/name):sha(out/name) for name in ['run.mjs','editor.tsx','observe-library.js','check.js','reload-check.js','check-config.json','runtime-authority.json','runtime-inputs.json','source-snapshot.json']};write(out/'inputs.json',inputs)
write(out/'request.json',dict(status='READY_FOR_SOLE_BROWSER_OWNER',command='node '+rel(out/'run.mjs'),scope='Tree08 approved wood; exact native033 gameplay/placement; canopy excluded',stage_proof_sha256=sha(stage/'stage-proof.json'),equivalence_sha256=sha(R/'tree08-native33-export-v2/reviewed-export-equivalence.json'),inputs=inputs,canonical_writes=False,browser_launched=False))
print(out)

"""Freeze a private HTTP library snapshot for the exact seven-state visual suite."""
import hashlib,json,os,shutil,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];LIVE=ROOT/'level-editor/library'
expected,name=sys.argv[1:]
subprocess.run([sys.executable,str(Path(__file__).with_name('restart12_prepare_seven_visual_proof.py')),expected,name],check=True)
b=ROOT/'level-editor/work/croisement02-refinement/restart2-state'/name;snap=b/'snapshot/library';snap.mkdir(parents=True)
def sha(p):
 with p.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def freeze(src,dst):
 dst.parent.mkdir(parents=True,exist_ok=True)
 if src.suffix.lower() in ['.json','.gltf','.txt','.csv']:shutil.copyfile(src,dst)
 else:os.link(src,dst)
for folder in ['3d-assets','game-data','mission-states','scenes']:
 for directory,dirs,files in os.walk(LIVE/folder):
  dirs[:]=[d for d in dirs if not d.startswith('.')and d!='backups']
  for filename in files:
   src=Path(directory)/filename
   if src.is_file()and not filename.startswith('.'):freeze(src,snap/src.relative_to(LIVE))
assert sha(snap/'scenes/croisement02.rhlos-map.json')==expected
raw=json.loads((snap/'scenes/croisement02.rhlos-map.json').read_text())
for row in raw['assetSources']+raw['sceneAssets']:
 for key in ['model','descriptor']:assert sha(snap/row[key])==row[key+'_sha256'],row['id']
index=json.loads((snap/'3d-assets/index.json').read_text())
for entry in index['assets']:assert sha(snap/'3d-assets'/entry['descriptor'])==entry['descriptor_sha256'],entry['id']
(snap/'scenes/index.json').write_text(json.dumps(sorted(p.name for p in(snap/'scenes').glob('*.rhlos-map.json')))+'\n')
shutil.copyfile(snap/'mission-states/index.json',b/'snapshot/installed34-index.json')
pkg=ROOT/'level-editor/work/croisement02-refinement/restart2-state/remaining-seven-package-v2';manifest=json.loads((pkg/'manifest.json').read_text())
for row in manifest['files']+[manifest['private_index']]:
 src=pkg/'library'/row['path'];assert sha(src)==row['sha256'];dst=snap/row['path'];dst.parent.mkdir(parents=True,exist_ok=True)
 if dst.exists():
  if row['path']!='mission-states/index.json':assert sha(dst)==row['sha256'];continue
  dst.unlink() # Only the private copied snapshot index; live index is untouched.
 freeze(src,dst)
# Contact pins use the same frozen static and state resources.
p=b/'contacts/manifest.json';s=p.read_text().replace(str(pkg/'library'),str(snap)).replace(str(LIVE),str(snap));p.write_text(s)
p=b/'editor.tsx';p.write_text(p.read_text().replace('await openHttpGameData()','await openHttpGameData("/seven-library/game-data/")'))
p=b/'states.mjs';s=p.read_text().replace("readFile('level-editor/library/mission-states/index.json')",'readFile('+json.dumps(str(b/'snapshot/installed34-index.json'))+')');p.write_text(s)
p=b/'run.mjs';s=p.read_text();s=s.replace("const stateRoot=join(root,'level-editor/work/croisement02-refinement/restart2-state/remaining-seven-package-v2/library');",'const stateRoot='+json.dumps(str(snap))+';')
s=s.replace("const manifest=JSON.parse(await readFile(join(stateRoot,'../manifest.json'),'utf8'));\nconst overlay=new Set(manifest.files.map(row=>row.path));overlay.add('mission-states/index.json');",'')
s=s.replace("if(!overlay.has(relative)){res.writeHead(307,{Location:'/library/'+relative}).end();return}","// Every library read is snapshot-only; absent resources never fall back to mutable live files.")
s=s.replace("readFile(join(stateRoot,relative)).then(bytes=>{", "readFile(join(stateRoot,relative)).then(async bytes=>{await writeFile(join(out,'served-snapshot.jsonl'),JSON.stringify({path:relative,sha256:sha(bytes)})+'\\n',{flag:'a'});")
s=s.replace("}).catch(next);", "}).catch(error=>{if(error.code==='ENOENT')res.writeHead(404).end();else next(error)});")
s=s.replace(" await verifyPins();\n await writeFile(join(out,'verification.json')", " await verifyPins();\n const liveMapSha=sha(await readFile(join(root,'level-editor/library/scenes/croisement02.rhlos-map.json')));await writeFile(join(out,'live-drift.json'),JSON.stringify({tested_snapshot_map_sha256:inputs.static_map_sha256,live_map_sha256:liveMapSha,changed:liveMapSha!==inputs.static_map_sha256,scope:'Snapshot proof does not establish correctness of later live changes.'},null,2));\n await writeFile(join(out,'verification.json')")
s=s.replace("relative.endsWith('.json')?'application/json':relative.endsWith('.png')?'image/png':'model/gltf-binary'","relative.endsWith('.json')?'application/json':relative.endsWith('.png')?'image/png':relative.endsWith('.avif')?'image/avif':relative.endsWith('.webp')?'image/webp':relative.endsWith('.jpg')?'image/jpeg':'model/gltf-binary'")
p.write_text(s)
p=b/'inputs.json';inputs=json.loads(p.read_text());inputs['files']={path:digest for path,digest in inputs['files'].items()if not path.startswith('level-editor/library/')}
for path in (b/'snapshot').rglob('*'):
 if path.is_file():inputs['files'][str(path.relative_to(ROOT))]=sha(path)
for path in [b/'editor.tsx',b/'states.mjs',b/'run.mjs',b/'contacts/manifest.json',Path(__file__).resolve()]:inputs['files'][str(path.relative_to(ROOT))]=sha(path)
inputs['scope']='Exact frozen full HTTP library snapshot plus seven approved private entries. Runtime and all snapshot bytes pinned before/after; concurrent live metadata changes are separately reported, never claimed tested.'
p.write_text(json.dumps(inputs,indent=2)+'\n')
(b/'snapshot-manifest.json').write_text(json.dumps({'status':'FROZEN_PRIVATE_NOT_RUN','map_sha256':expected,'files':sum(1 for p in snap.rglob('*')if p.is_file()),'metadata':'Copied small mutable manifests/descriptors','models':'Hardlinks to immutable binary resources; all SHA-pinned before/after','inputs_sha256':sha(p),'canonical_writes':False},indent=2)+'\n')
print(b)

"""Derive a frozen-code seven-state proof from an immutable private library snapshot."""
import hashlib,json,os,shutil,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];old=Path(sys.argv[1]).resolve();b=old.with_name(sys.argv[2]);assert not b.exists();assert old.parent==ROOT/'level-editor/work/croisement02-refinement/restart2-state'
def sha(p):
 with p.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
for directory,dirs,files in os.walk(old):
 dirs[:]=[d for d in dirs if d not in ['browser','runtime-source']]
 for name in files:
  src=Path(directory)/name;rel=src.relative_to(old)
  if str(rel)in ['prelaunch-runtime-drift.json']:continue
  dst=b/rel;dst.parent.mkdir(parents=True,exist_ok=True)
  if src.suffix in ['.json','.mjs','.ts','.tsx','.html','.css','.txt']:
   dst.write_text(src.read_text().replace(str(old),str(b)).replace(str(old.relative_to(ROOT)),str(b.relative_to(ROOT))))
  else:os.link(src,dst)
runtime=b/'runtime-source/level-editor';live_runtime={}
for folder in ['app/src','shared/src']:
 for src in(ROOT/'level-editor'/folder).rglob('*'):
  if src.is_file():
   dst=runtime/src.relative_to(ROOT/'level-editor');dst.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(src,dst);live_runtime[str(src.relative_to(ROOT))]=sha(dst)
for name in ['tsconfig.base.json','package.json','app/vite.config.ts','app/package.json','app/tsconfig.json','app/tests/cdp.mjs','shared/package.json','shared/tsconfig.json']:
 src=ROOT/'level-editor'/name
 if src.exists():
  dst=runtime/name;dst.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(src,dst);live_runtime[str(src.relative_to(ROOT))]=sha(dst)
for name in ['missions.ts','types.ts']:
 src=ROOT/'wasm-www/src/leaderboards'/name;dst=b/'runtime-source'/src.relative_to(ROOT);dst.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(src,dst);live_runtime[str(src.relative_to(ROOT))]=sha(dst)
(b/'node_modules').symlink_to((ROOT/'level-editor/app/node_modules').resolve(),target_is_directory=True)
for name in ['app/node_modules','shared/node_modules','node_modules']:
 src=ROOT/'level-editor'/name
 if src.exists():
  dst=runtime/name;dst.parent.mkdir(parents=True,exist_ok=True);dst.symlink_to(src.resolve(),target_is_directory=True)
for p in [b/'editor.tsx',b/'contacts/proof.ts']:
 s=p.read_text()
 for folder in ['app/src','shared/src']:s=s.replace(str(ROOT/'level-editor'/folder),str(runtime/folder))
 p.write_text(s)
p=b/'run.mjs';s=p.read_text().replace((ROOT/'level-editor/app/tests/cdp.mjs').as_uri(),(runtime/'app/tests/cdp.mjs').as_uri()).replace("configFile:join(root,'level-editor/app/vite.config.ts')",'configFile:'+json.dumps(str(runtime/'app/vite.config.ts'))).replace("root:join(root,'level-editor/app')",'root:'+json.dumps(str(runtime/'app'))).replace("server:{host:'127.0.0.1'","server:{fs:{allow:[root]},host:'127.0.0.1'")
s=s.replace('configFile:', 'resolve:{alias:{"@rle/shared":'+json.dumps(str(runtime/'shared/src/index.ts'))+'}},configFile:',1)
needle=" const liveMapSha=sha(await readFile(join(root,'level-editor/library/scenes/croisement02.rhlos-map.json')));"
replacement=" const runtimeBaseline=JSON.parse(await readFile(join(base,'runtime-baseline.json'),'utf8'));const runtimeDrift=[];for(const[path,expected]of Object.entries(runtimeBaseline.source_files)){const actual=sha(await readFile(join(root,path)));if(actual!==expected)runtimeDrift.push({path,expected,actual})}await writeFile(join(out,'live-runtime-drift.json'),JSON.stringify({changed:runtimeDrift.length>0,files:runtimeDrift,scope:'Snapshot proof does not validate later live runtime changes.'},null,2));\n"+needle
assert needle in s;s=s.replace(needle,replacement);p.write_text(s)
p=b/'inputs.json';inputs=json.loads(p.read_text());inputs['files']={n:h for n,h in inputs['files'].items()if not n.startswith(('level-editor/app/src/','level-editor/shared/src/'))and n!='level-editor/app/vite.config.ts'}
for path in b.rglob('*'):
 if path.is_file()and 'node_modules'not in path.parts and path.name not in ['inputs.json','snapshot-manifest.json']:
  inputs['files'][str(path.relative_to(ROOT))]=sha(path)
baseline={'source_revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'source_files':live_runtime,'scope':'Copied exact app/shared source; immutable dependencies reuse installed node_modules. Source differences at finish are reported separately.'}
q=b/'runtime-baseline.json';q.write_text(json.dumps(baseline,indent=2)+'\n');inputs['files'][str(q.relative_to(ROOT))]=sha(q);inputs['files'][str(Path(__file__).resolve().relative_to(ROOT))]=sha(Path(__file__).resolve());inputs['scope']='Frozen complete HTTP library and copied app/shared runtime. Same production adapter and full assertions; no mutable library/source fallback. Later live changes require separate compatibility review.';p.write_text(json.dumps(inputs,indent=2)+'\n')
q=b/'snapshot-manifest.json';v=json.loads(q.read_text());v['inputs_sha256']=sha(p);v['runtime_baseline_sha256']=sha(b/'runtime-baseline.json');v['source_revision']=baseline['source_revision'];q.write_text(json.dumps(v,indent=2)+'\n');print(b)

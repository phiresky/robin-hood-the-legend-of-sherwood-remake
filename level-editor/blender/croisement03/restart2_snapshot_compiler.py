"""Copy an isolated compiler and dependencies; never link to mutable source files."""
import argparse,hashlib,json,shutil,os
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 ap=argparse.ArgumentParser();ap.add_argument('output',type=Path);ap.add_argument('--complete-incomplete-copy',action='store_true');a=ap.parse_args();out=a.output.resolve();assert not (out/'snapshot.json').exists();out.mkdir(parents=True,exist_ok=a.complete_incomplete_copy)
 sources=[]
 for package in ['','shared','pipeline','app']:
  base=ROOT/'level-editor'/package
  for name in ['src','node_modules','package.json','tsconfig.json']:
   p=base/name
   if p.exists() and (name!='src' or package):sources.append(p)
 sources += [ROOT/'level-editor/pnpm-lock.yaml',ROOT/'level-editor/pnpm-workspace.yaml']
 helpers=['restart2_trio_compiler_guard.mjs','restart2_validate_final_trio.mjs','restart2_run_frozen_compiler.mjs']
 sources += [ROOT/'level-editor/blender/croisement03'/n for n in helpers]
 original={}
 for p in sources:
  files=[p] if p.is_file() else [x for x in p.rglob('*') if x.is_file() and not x.is_symlink()]
  for f in files:original[str(f.relative_to(ROOT))]=digest(f)
  target=out/p.relative_to(ROOT);target.parent.mkdir(parents=True,exist_ok=True)
  if p.is_dir():
   for directory,dirs,names in os.walk(p,followlinks=False):
    src=Path(directory);dst=out/src.relative_to(ROOT);dst.mkdir(parents=True,exist_ok=True)
    for name in dirs+names:
     f=src/name;t=dst/name
     if f.is_symlink():
      if t.is_symlink():assert t.readlink()==f.readlink()
      else:t.symlink_to(f.readlink())
     elif f.is_file() and (not t.exists() or digest(t)!=original[str(f.relative_to(ROOT))]):shutil.copy2(f,t)
  else:shutil.copy2(p,target)
 pins={};links={}
 for p in out.rglob('*'):
  if p.is_symlink():
   assert p.resolve().is_relative_to(out),f'Escaping dependency link: {p}'
   if not p.exists():assert not (ROOT/p.relative_to(out)).exists(),'New broken dependency link'
   links[str(p.relative_to(out))]=str(p.readlink())
  elif p.is_file():pins[str(p.resolve())]=digest(p)
 for name,h in original.items():
  assert pins[str(out/name)]==h,f'Copied bytes differ: {name}'
 receipt={'status':'FROZEN_COMPILER_COPY_VERIFIED','capture':'Each copied file matches its captured source bytes; live sources may subsequently change. This is an executed snapshot, not a claim of one atomic repository revision.','scope':'Asset-only equivalence under this copied compiler, not latest compiler or final game verification.','root':str(out),'sourceRoot':str(ROOT),'compilerPins':pins,'links':links,'sourcePins':original}
 (out/'snapshot.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps({'path':str(out/'snapshot.json'),'sha256':digest(out/'snapshot.json'),'files':len(pins)}))
if __name__=='__main__':main()

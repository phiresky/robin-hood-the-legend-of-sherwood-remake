"""Prepare canonical trio editor verification without launching a browser."""
import json,hashlib,shutil,subprocess
from pathlib import Path
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';OLD=B/'combined-trio-editor-interaction-browser-v1';O=B/'combined-trio-canonical-postinstall-browser-v2';LIB=R/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 assert not O.exists();O.mkdir();oldlib=json.loads((OLD/'check-config-v4.json').read_text())['library'];replace=lambda s:s.replace(str(OLD.relative_to(R)),str(O.relative_to(R))).replace(oldlib,str(LIB))
 cfg=json.loads((OLD/'check-config-v4.json').read_text());cfg=json.loads(replace(json.dumps(cfg)));mp=LIB/'scenes/croisement03.rhlos-map.json';idx=LIB/'3d-assets/index.json';cfg['stage_map']=json.loads(mp.read_text());cfg['map_sha256']=sha(mp);cfg['catalog_sha256']=sha(idx)
 for aid,a in cfg['assets'].items():
  for key in ('model','descriptor'):
   assert sha(LIB/a[key])==a[key+'_sha256'],(aid,key)
 write(O/'check-config-v4.json',cfg)
 for name in ['editor.tsx','observe-library.js','check-v6.js']:(O/name).write_text(replace((OLD/name).read_text()))
 reloadp=B/'combined-trio-inferred-crown-candidate-v1/editor-interaction-fixture-v5/reload-check.js';shutil.copy2(reloadp,O/'reload-check.js')
 script=replace((OLD/'run-v6.mjs').read_text()).replace(str(reloadp.relative_to(R)),str((O/'reload-check.js').relative_to(R))).replace('/browser-v6','/browser-v1')
 (O/'run-v1.mjs').write_text(script)
 authority=json.loads((OLD/'runtime-authority.json').read_text());snapshot={}
 for directory in authority['sourceDirectories']:
  for p in sorted((R/directory).rglob('*')):
   if p.is_file():
    rel=p.relative_to(R);dest=O/'snapshot'/rel;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,dest);snapshot[str(rel)]=sha(dest)
 authority['membership']=sorted(snapshot);catalog=json.loads(idx.read_text());subset=sorted((a for a in catalog['assets']if a.get('source_map','').lower()=='croisement03'),key=lambda a:a['id']);canonical_hash=subprocess.check_output(['node','--input-type=module','-e',"import fs from 'node:fs';import c from 'node:crypto';const stable=x=>Array.isArray(x)?x.map(stable):x&&typeof x==='object'?Object.fromEntries(Object.keys(x).sort().map(k=>[k,stable(x[k])])):x;const rows=JSON.parse(fs.readFileSync(process.argv[1])).assets.filter(x=>x.source_map?.toLowerCase()==='croisement03').sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0);console.log(c.createHash('sha256').update(JSON.stringify(stable(rows))).digest('hex'));",str(idx)],text=True).strip()
 authority['index']={'path':str(idx),'initial_sha256':sha(idx),'c02_count':len(subset),'c02_sha256':canonical_hash};write(O/'runtime-authority.json',authority);write(O/'source-snapshot.json',snapshot)
 oldpins=json.loads((OLD/'runtime-inputs.json').read_text());pins={}
 for path in oldpins:
  new=replace(path);p=Path(new);actual=p if p.is_absolute()else R/p
  pins[new]=snapshot.get(new)or sha(actual)
 pins[str(mp)]=sha(mp);write(O/'runtime-inputs.json',pins)
 private={str(p):sha(p)for p in [O/n for n in ['editor.tsx','observe-library.js','check-config-v4.json','check-v6.js','reload-check.js','runtime-authority.json','runtime-inputs.json','source-snapshot.json','run-v1.mjs']]};write(O/'inputs-v6.json',private)
 # The catalog subset hash must match JavaScript's recursively sorted JSON exactly.
 write(O/'request.json',{'status':'PREPARED_NOT_EXECUTED','cwd':str(R),'argv':['node',str(O/'run-v1.mjs')],'canonical_library':str(LIB),'map_sha256':sha(mp),'source_files':len(snapshot),'expected_checks':167,'no_browser_launched':True,'prior_interaction_handoff':str(OLD/'handoff.json'),'prior_interaction_sha256':sha(OLD/'handoff.json'),'inputs':private,'scope':'Canonical installed trio selection, palette insertion, save and full reload against newly frozen source. Appearance and native14phase proof linked separately.'})
 print(O)
if __name__=='__main__':main()

"""Deduplicate warnings privately, preserving every distinct gameplay warning and all other fields."""
import hashlib,json,shutil
from pathlib import Path
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2/combined-trio-stage-v2';L=B/'library';M=L/'scenes/croisement03.rhlos-map.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not (B/'draft-normalization.json').exists(), 'Stage already normalized; preserve its evidence receipt'
 doc=json.loads(M.read_text());rows=[]
 for ref in doc['assetSources']+doc['sceneAssets']:
  a=json.loads((L/ref['descriptor']).read_text());issues=a.get('gameplay',{}).get('draft',{}).get('issues',[])
  assert all(isinstance(x,str) and x.strip() for x in issues)
 for ref in doc['assetSources']+doc['sceneAssets']:
  p=L/ref['descriptor'];a=json.loads(p.read_text());issues=a.get('gameplay',{}).get('draft',{}).get('issues',[])
  if len(issues)==len(set(issues)):continue
  assert all(isinstance(x,str) and x.strip() for x in issues)
  source=p.resolve();before=sha(source);unique=list(dict.fromkeys(issues));a['gameplay']['draft']['issues']=unique
  directory=p.parent
  if directory.is_symlink():
   target=directory.resolve();directory.unlink();directory.mkdir()
   for child in target.iterdir():
    if child.name!='asset.json':(directory/child.name).symlink_to(child.resolve(),target_is_directory=child.is_dir())
  else:
   source=B/'descriptor-before-normalization'/ref['id']/'asset.json';source.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,source)
  p.write_text(json.dumps(a,indent=2)+'\n');ref['descriptor_sha256']=sha(p);assert sha(source)==before
  rows.append(dict(asset=ref['id'],source=str(source),source_sha256=before,staged_descriptor=str(p),staged_sha256=sha(p),original_issues=issues,retained_issues=unique,scope='Duplicate warning strings only; no warning removed and gameplay geometry unchanged'))
 M.write_text(json.dumps(doc,indent=2)+'\n');(B/'draft-normalization.json').write_text(json.dumps(dict(changes=rows,map_sha256=sha(M)),indent=2)+'\n');print(json.dumps(rows,indent=2))
if __name__=='__main__':main()

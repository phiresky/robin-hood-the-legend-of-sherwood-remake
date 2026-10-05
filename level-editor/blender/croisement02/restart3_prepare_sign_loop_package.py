"""Stage a source-pinned sign loop without copying already installed artwork."""
import json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/croisement02-refinement'
LIB=ROOT/'level-editor/library'
DEST=WORK/'restart2-state/mission-sign-loop-package-v1'
sha=lambda b:hashlib.sha256(b).hexdigest()
def main():
 if DEST.exists():raise FileExistsError(DEST)
 DEST.mkdir();records={};existing={}
 def write(relative,data,source):
  target=LIB/relative;digest=sha(data)
  if target.exists() and target.read_bytes()==data:
   existing[relative]={'path':relative,'sha256':digest,'bytes':len(data)}
  else:
   path=DEST/'library'/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data);records[relative]={'path':relative,'sha256':digest,'bytes':len(data),'source':str(source)}
  return {'path':relative,'sha256':digest}
 def walk(value):
  if isinstance(value,list):
   for item in value:walk(item)
  elif isinstance(value,dict):
   if isinstance(value.get('path'),str) and 'sha256'in value:
    source=WORK/value['path'];data=source.read_bytes();assert sha(data)==value['sha256'];value['path']=write('mission-states/croisement02/resources/'+sha(data)+source.suffix.lower(),data,source)['path']
   for item in value.values():walk(item)
 source=WORK/'restart2-state/native-art-browser-v1/contract.json';native=json.loads(source.read_text());walk(native)
 contract={'version':1,'scope':'controlled-native-loop-preview','native':native,'focus_element_id':'sign4'}
 entry={'id':'signposts','name':'Signposts and ambient animation','kind':'native-loop','map':'Croisement02','mission':'S03_FoB_MP'}
 for key,name in [('mission_data','S03_FoB_MP.rhm.json'),('level_data','Croisement02.rhp.json')]:
  path=LIB/'game-data/Data/Levels'/name;entry[key]=write('mission-states/croisement02/source/'+name,path.read_bytes(),path)
 entry['contract']=write('mission-states/croisement02/contracts/signposts.json',(json.dumps(contract,indent=2)+'\n').encode(),source)
 current=LIB/'mission-states/index.json';index=json.loads(current.read_text());assert not any(e['id']=='signposts'for e in index['entries']);index['entries'].append(entry);write('mission-states/index.json',(json.dumps(index,indent=2)+'\n').encode(),current)
 report={'status':'Private incremental sign loop package; actual-editor verification required before installation','previous_index_sha256':sha(current.read_bytes()),'source_contract_sha256':sha(source.read_bytes()),'entry':entry,'new_files':list(records.values()),'existing_files':list(existing.values()),'new_bytes':sum(r['bytes']for r in records.values()),'scope':'Original artwork only; exact sign/ambient source timing and ordering. No physical occlusion or gameplay-script completion claim.'};(DEST/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(len(records),'new',len(existing),'reused',report['new_bytes'],'bytes')
if __name__=='__main__':main()

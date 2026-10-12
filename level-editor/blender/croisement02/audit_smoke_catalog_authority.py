"""Accept only descriptor-backed unrelated metadata drift for a canonical smoke."""
import hashlib,json
from pathlib import Path
from rebase_round_wood_index import rows,require
sha=lambda b:hashlib.sha256(b).hexdigest()

def audit_catalog(published,current,asset_root):
 a,b=rows(published),rows(current)
 require(list(a)==list(b),'Catalog membership/order drift')
 protected=[];changes=[]
 for aid in a:
  old,new=json.loads(a[aid][2]),json.loads(b[aid][2])
  owned=False
  for row in (old,new):
   source=row.get('source_map');require(isinstance(source,str),'Invalid source_map')
   owned|=source.casefold()=='croisement02' or row['id'].casefold().startswith('croisement02-')
  if owned:
   protected.append(aid);require(a[aid][2]==b[aid][2],'Owned C02 row drift: '+aid)
  elif a[aid][2]!=b[aid][2]:
   fields=sorted(k for k in set(old)|set(new)if old.get(k)!=new.get(k))
   require(set(fields)<={'editor','descriptor_sha256'},'Unsupported unrelated change: '+aid)
   descriptor=(asset_root/new['descriptor']).resolve();require(descriptor.is_relative_to(asset_root.resolve()),'Descriptor outside library')
   raw=descriptor.read_bytes();require(sha(raw)==new['descriptor_sha256'],'Unbacked descriptor: '+aid)
   obj=json.loads(raw);require(obj['id']==aid and obj['source_map']==new['source_map'],'Descriptor identity mismatch')
   editor=new.get('editor');require(isinstance(editor,dict),'Missing embedded editor')
   require(all(obj.get(k)==v for k,v in editor.items()),'Embedded editor disagrees with descriptor: '+aid)
   changes.append({'id':aid,'fields':fields,'descriptor':{'path':str(descriptor),'sha256':sha(raw)},'old_row_sha256':sha(a[aid][2]),'new_row_sha256':sha(b[aid][2]),'editor_changed_fields':sorted(k for k in set(old.get('editor',{}))|set(editor)if old.get('editor',{}).get(k)!=editor.get(k))})
 return {'status':'PASS_OWNED_ROWS_EXACT_UNRELATED_METADATA_DESCRIPTOR_BACKED','published_index_sha256':sha(published),'current_index_sha256':sha(current),'owned_rows':protected,'changes':changes,'limitations':'Current unrelated rows retained and pinned, not newly behavior-certified. No catalog writes.'}

def published_authority(plan,library):
 records=plan['records'];require(len(records)==12,'Expected12 publication targets')
 index=library/'3d-assets/index.json';entry=next(r for r in records if Path(r['target'])==index)
 require(sha(Path(entry['source']).read_bytes())==entry['new_sha256'],'Published candidate drift')
 for rec in records:
  if rec is not entry:require(sha(Path(rec['target']).read_bytes())==rec['new_sha256'],'Owned published resource drift: '+rec['target'])
 for pin in plan['protected_descriptors']:
  path=Path(pin['path']);require(sha(path.read_bytes())==pin['sha256'],'Protected descriptor drift')
 current=index.read_bytes();audit=audit_catalog(Path(entry['source']).read_bytes(),current,library/'3d-assets')
 require(sha(index.read_bytes())==audit['current_index_sha256'],'Index drift during audit')
 audit['owned_non_index_targets']=11
 return audit

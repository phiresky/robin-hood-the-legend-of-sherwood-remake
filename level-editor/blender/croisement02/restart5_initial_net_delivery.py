"""Freeze reviewed initial-rig derivatives and ten source-instance bindings."""
import json,hashlib,struct,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
BASE=OUT/'restart5-initial-nets'
DEST=BASE/'approved-delivery-v2'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 assert shutil.disk_usage(OUT).free>23*1024**3
 assert not(DEST/'manifest.json').exists()
 records=[]
 for key in ['00','01']:
  folder=DEST/f'profile-{key}';export=json.loads((folder/'export.json').read_text());glb=folder/'model.glb';assert sha(glb)==export['glb_sha256'];assert sha(export['model_source'])==export['model_sha256'];geometry=json.loads((folder/'geometry-guard.json').read_text());assert geometry['status']=='PASS'and geometry['export_glb_sha256']==sha(glb);review=json.loads((folder/'root-review.json').read_text());assert review['status']=='PASS'and review['glb_sha256']==sha(glb)
  b=glb.read_bytes();n,kind=struct.unpack_from('<II',b,12);doc=json.loads(b[20:20+n]);binary=b[28+n:];images=[]
  for image in doc['images']:
   v=doc['bufferViews'][image['bufferView']];data=binary[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']];p=folder/(image['name']+'.png');assert hashlib.sha256(data).hexdigest()==sha(p);images.append(dict(image=image['name'],packed_png_exact=True,sha256=sha(p)))
  assert all(s['magFilter']==9728 and s['minFilter']==9984 for s in doc['samplers'])
  files={str(p):sha(p)for p in [glb,folder/'export.json',folder/'geometry-guard.json',folder/'root-review.json',folder/'agent-review.json',folder/'render-comparison.json',folder/'actual/textured.png',folder/'native-parent-comparison.png',folder/'underside/sheet.png',folder/'underside/evidence.json']}
  records.append(dict(asset_id=export['asset_id'],profile=export['profile'],glb=str(glb),glb_sha256=sha(glb),approved_model=export['model_source'],approved_model_sha256=export['model_sha256'],family=export['family'],position=export['position'],parts=export['parts'],native_exact_RGBA=export['native_exact_RGBA'],images=images,evidence=files,limits=export['limits']))
 authority=OUT/'restart2-state/net-initial-action-audit-v1/report.json';source=json.loads(authority.read_text());instances=[]
 for row in source['instances']:
  target=row['target'];assert target['profile_name']in ['Croisement02 - piege01h','Croisement02 - piege03h'];key='00'if target['profile_name']=='Croisement02 - piege01h'else'01';record=records[int(key)];assert target['action']==target['direction']==0
  instances.append(dict(mission=row['mission'],target_index=row['target_index'],native_profile=target['profile_name'],profile=record['profile'],asset_id=record['asset_id'],initial_glb=record['glb'],initial_glb_sha256=record['glb_sha256'],position=record['position'],display_anchor=[target['position_x'],target['position_y']],action_position_unchanged=[target['action_position_x'],target['action_position_y']],source_target_sha256=hashlib.sha256(json.dumps(target,sort_keys=True,separators=(',',':')).encode()).hexdigest(),scope='Initial action0 only. Existing e/i endpoints, trigger scripts, source target and gameplay remain unchanged.'))
 assert len(instances)==10
 total=sum(p.stat().st_size for d in BASE.glob('approved-delivery-v*')for p in d.rglob('*')if p.is_file());assert total<50*1024**2
 approval=OUT/'restart3-review-batches/batch-v10/user-approval.json';manifest=dict(status='PASS independently reviewed approved-model delivery',approval=str(approval),approval_sha256=sha(approval),source_instance_authority=str(authority),source_instance_authority_sha256=sha(authority),records=records,instances=instances,new_aggregate_bytes=total,scope='Private derivatives only; state owner installs exact initial bindings. No live catalog, scene, transition or gameplay writes.')
 (DEST/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');assert shutil.disk_usage(OUT).free>23*1024**3;print(sha(DEST/'manifest.json'))
if __name__=='__main__':main()

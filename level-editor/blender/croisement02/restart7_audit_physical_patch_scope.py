"""Record source-state coverage separately from physical endpoint readiness."""
from pathlib import Path
import collections,hashlib,json
import numpy as np
from PIL import Image
B=Path('level-editor/work/croisement02-refinement'); P=B/'restart7-source-patch-delivery/contracts-v1'; O=B/'restart7-source-patch-delivery/physical-scope-v1';O.mkdir(exist_ok=True)
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
manifest=json.loads((P/'manifest.json').read_text());resources={r['path']:Path(r['source']) for r in manifest['resources']}; grouped=collections.defaultdict(list)
static=B/'restart2-textures/post-batch15-static-candidate-v2/croisement02.rhlos-map.json';scene=json.loads(static.read_text())
for r in manifest['records']:grouped[r['profile']].append(r)
records=[]
for name,rows in grouped.items():
 instances=[]
 for r in rows:
  c=json.loads((P/r['contract']).read_text()); p=next(x for x in c['native']['patch_states'] if x['id']==c['focus_patch_id']);instances.append({'id':r['id'],'contract':r['contract'],'contract_sha256':sha(P/r['contract']),'display_position':p['display_position'],'elevation':p['elevation'],'initial':p['initial'],'terminal':p['transition'][-1] if p['integrate_in_background'] else (p['final'][0] if p['final'] else None),'integrates_last_transition':p['integrate_in_background'],'physical_binding_present':bool(c.get('families') or c.get('physical'))})
 if 'hidden archer' in name:
  finding='Source endpoints visibly change foliage coverage and expose substrate between trunks/branches. Existing static foliage remains unchanged by these source-only controls.'
  gap='Classify actual foreground foliage fragments and exposed receiver at each of the five fixed profile locations; prepare approved initial/applied foliage and substrate states, with same-world source comparison and oblique contact. Do not replace trunks or entire crowns based only on patch alpha.'
 elif name.endswith('hole'):
  finding='Initial leaf camouflage becomes an open dark recess/rim. No dedicated physical cover/recess asset is present in the static asset inventory.'
  gap='Construct reusable initial leaf-cover and applied rim/recess endpoints; bind each instance to its actual ground/bank receiver. Hidden depth is inferred; preserve source-visible leaves and opening. No continuous collapse motion required.'
 elif name.endswith('hiding Pc'):
  finding='Source endpoints change vegetation/soil appearance. Instances include elevated foreground and ground-level bindings, so one flat ground decal cannot be assumed correct for all32.'
  gap='Partition instances by actual ground/bank/foliage receiver; apply terminal source appearance to appropriate physical surfaces, and alter foreground foliage only where endpoint visibility requires it. Retain underlying static base and phase precedence.'
 else:
  finding='This orphan net effect is not an occupied/empty bag endpoint. Source final-row/absence must be evaluated independently; existing approved net profiles do not establish its physical initial or terminal binding.'
  gap='Audit exact net-source rows and same-location initial rigging compatibility. Reuse approved initial geometry only if matching source/placement is proven; do not fabricate a permanent bag from an effect profile.'
 records.append({'profile':name,'instances':len(rows),'source_control_status':'PRIVATE_SOURCE_PARITY_PASS_UI_PENDING','physical_status':'OPEN_NO_ENDPOINT_BINDING_OR_SAVED_SCENE_STATE_PROOF','observed_requirement':finding,'finite_next_task':gap,'members':instances})
report={'scope':'Eight-profile physical reveal gap inventory, not a physical render pass. Source review establishes endpoint artwork differences; static inventory and wrappers establish absence of their controlled physical bindings. Receiver identity must be independently audited before geometry or material changes.','static_map':str(static),'static_map_sha256':sha(static),'static_assets':len(scene['assetSources'])+len(scene['sceneAssets']),'source_manifest_sha256':sha(P/'manifest.json'),'source_review':str(P/'source-review-v1/manifest.json'),'source_review_sha256':sha(P/'source-review-v1/manifest.json'),'denominators':{'mission_patches':129,'live_native_controls':43,'reviewed_private_existing_controls':47,'remaining_private_source_controls':82,'combined_private_source_controls':129,'physical_completed_profiles_among_these_eight':0,'physical_completion_not_inferred':True},'records':records,'static_state_only_metadata_note':'Four state-only obstacle groups in the static catalog are not substitutes for reviewed visible patch endpoints.','approval_scope':'Source-only grouped review cannot approve missing physical endpoint geometry or receiver appearance.'}
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(O/'report.json');print(sha(O/'report.json'))

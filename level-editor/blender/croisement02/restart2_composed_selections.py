"""Strict dual-source crown and wood composition selection after independent review."""
import json,math
from pathlib import Path
from evidence_io import sha,write_json
from catalog_schema import source_for_part

def read(p):return json.loads(Path(p).read_text())
def require(ok,why):
 if not ok:raise ValueError(why)

def validate(worker):
 digest=sha(worker/'model.blend');proof=read(worker/'inspection/crown-wood-composition.json')
 first=read(worker/'modified/views.json')['views'][0];matrix=first['camera_matrix_world'];direction=[matrix[i][2] for i in range(3)];length=math.sqrt(sum(v*v for v in direction));native=[0,-math.cos(math.radians(35)),math.sin(math.radians(35))]
 require(first['ortho_scale']>0 and sum(a*b for a,b in zip(direction,native))/length>.999999,'Combined first view is not native orthographic direction')
 require(proof['model_sha256']==digest and proof['non_crown_geometry_appearance_exact'],'Stale composition proof')
 require(proof['crown_before']==proof['crown_after'] and proof['wood_before']==proof['wood_after'],'Composition changed either source component')
 for kind in ('wood','crown'):require(sha(Path(proof[kind+'_worker'])/'model.blend')==proof[kind+'_model_sha256'],'Frozen composition input changed')
 require(sha(Path(proof['crown_worker'])/'inspection/envelope-preservation.json')==proof['crown_proof_sha256'],'Crown source proof changed')
 require(sha(worker/'source-masks.json')==proof['source_mask_sha256'],'Composition ownership changed')
 for name in ('saved-model-audit.json','source-coverage/report.json','actual-materials/evidence.json','actual-materials/opacity-bounds.json'):
  require(read(worker/'inspection'/name)['model_sha256']==digest,'Stale combined verification')
 require(read(worker/'validation.json')['status']=='PASS' and read(worker/'inspection/saved-model-audit.json')['status']=='PASS','Invalid combined model')
 coverage=read(worker/'inspection/source-coverage/report.json');bounds=read(worker/'inspection/actual-materials/opacity-bounds.json')
 require(coverage['intersection_over_union']>=.95 and bounds['crowns'] and min(row['depth_width_ratio'] for row in bounds['crowns'])>=1,'Combined native coverage or depth failed')
 review=read(worker/'inspection/composed-root-review.json');require(review['model_sha256']==digest and review['status']=='PASS combined geometry for user review','Independent combined review required')
 return proof

def selected_workspace(out,mask,catalog_path):
 if mask not in (31,32,35,38,43,45,46):return None
 receipt=out/'restart2-wood/composed-selections'/f'tree-{mask:02}-gallery-v3.json'
 if not receipt.exists():receipt=out/'restart2-wood/composed-selections'/f'tree-{mask:02}-gallery-v2.json'
 if not receipt.exists():receipt=out/'restart2-wood/composed-selections'/f'tree-{mask:02}.json'
 if not receipt.exists():return None
 record=read(receipt)
 if 'previous_receipt_sha256' in record:
  previous=receipt.with_name(f'tree-{mask:02}-gallery-v2.json' if receipt.name.endswith('-v3.json') else f'tree-{mask:02}.json')
  require(sha(previous)==record['previous_receipt_sha256'],'Composed receipt history changed')
 worker=Path(record['worker']);require(record['approval']=='pending' and record['model_sha256']==sha(worker/'model.blend'),'Invalid composed selection')
 require(worker.name==f'croisement02-tree-{mask:02}','Wrong composed asset')
 group=next(g for g in read(catalog_path)['groups'] if g['id']==worker.name);require(set(record['part_ids'])=={source_for_part(p) for p in group['parts']},'Composed source ownership changed')
 for path,digest in record['files'].items():require(sha(Path(path))==digest,'Bound composed evidence changed: '+path)
 validate(worker)
 return worker

def bind(out,worker,catalog_path,review_paths,review_text):
 worker=worker.resolve();mask=int(worker.name.rsplit('-',1)[1]);receipt=out/'restart2-wood/composed-selections'/f'tree-{mask:02}.json';require(not receipt.exists(),'Preserve existing composed receipt');receipt.parent.mkdir(parents=True,exist_ok=True)
 review=worker/'inspection/composed-root-review.json';require(not review.exists(),'Preserve earlier independent review');write_json(review,dict(model_sha256=sha(worker/'model.blend'),reviewer='root independent',status='PASS combined geometry for user review',exact_review_text=review_text,approval='pending',texture_status='HOLD: unknown bark completion after geometry approval'))
 proof=validate(worker);wood_receipt=out/'restart2-wood/selections'/('tree-31-v2.json' if mask==31 else f'tree-{mask}-continuous-v1.json' if mask in (43,45,46) else f'tree-{mask}.json');old=read(wood_receipt);require(old['model_sha256']==proof['wood_model_sha256'],'Combined wood differs from selected scoped receipt')
 paths={Path(p) for p in review_paths};paths.update(Path(p) for p in old['files']);paths.add(wood_receipt)
 for name in ('model.blend','workspace.json','source-masks.json','validation.json','modified/views.json','modified/solid.png','modified/textured.png'):paths.add(worker/name)
 for name in ('crown-wood-composition.json','composed-root-review.json','refinement.json','saved-model-audit.json','source-coverage/report.json','source-coverage/difference.png','actual-materials/evidence.json','actual-materials/opacity-bounds.json','actual-materials/sheet.png'):paths.add(worker/'inspection'/name)
 crown=Path(proof['crown_worker']);paths.update([crown/'model.blend',crown/'inspection/envelope-preservation.json']);crown_input=Path(read(crown/'inspection/envelope-preservation.json')['source_worker']);paths.add(crown_input/'inspection/prototype-preservation.json')
 envelope=read(crown/'inspection/envelope-preservation.json')
 if 'parent_combined' in envelope:
  parent=envelope['parent_combined'];parent_worker=Path(parent['worker']);parent_proof=Path(parent['composition_proof']);prior=crown/'inspection/prior-crown-envelope.json'
  require(sha(parent_worker/'model.blend')==parent['model_sha256'] and sha(parent_proof)==parent['composition_sha256'],'Support correction parent changed')
  require(parent['retained_wood_model_sha256']==proof['wood_model_sha256'] and sha(prior)==envelope['prior_envelope_sha256'],'Support correction wood or prior crown changed')
  paths.update([parent_worker/'model.blend',parent_proof,prior])
  for path,digest in envelope.get('dependency_recipes',{}).items():
   require(sha(Path(path))==digest,'Support correction recipe changed');paths.add(Path(path))
 write_json(receipt,dict(kind='exact-crown-and-scoped-wood-composition',worker=str(worker),model_sha256=sha(worker/'model.blend'),part_ids=read(worker/'workspace.json')['part_ids'],approval='pending',texture_status='HOLD: unknown bark completion after geometry approval',files={str(p.resolve()):sha(p) for p in sorted(paths)},limitations=['Independent geometry review does not inherit prior user approval.','Gray unknown bark awaits authorized texture completion; only permitted native/Leicester references.','Crown and scoped wood retain separate source and inference receipts.']))
 require(selected_workspace(out,mask,catalog_path)==worker,'Combined selector validation failed')
 return receipt

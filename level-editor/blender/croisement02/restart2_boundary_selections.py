"""Strict scoped inferred-contour receivers, never whole-tree completion."""
import json
from pathlib import Path
from evidence_io import sha,write_json
from catalog_schema import source_for_part

def read(p):return json.loads(p.read_text())
def require(ok,reason):
 if not ok:raise ValueError(reason)

def selected_workspace(out,mask,catalog_path):
 if mask not in (35,43,45,46):return None
 path=out/'restart2-wood/selections'/f'tree-{mask}.json'
 if not path.exists():return None
 record=read(path);worker=Path(record['worker']);digest=sha(worker/'model.blend');inspection=worker/'inspection'
 require(record['kind']=='scoped-inferred-wood-boundary' and record['mask']==mask,'Wrong boundary selection')
 require(record['approval']=='pending' and record['whole_tree_status']=='HOLD: legacy crown refinement and texture completion','Boundary receipt cannot approve whole tree')
 require(worker.name==record['asset_id']==f'croisement02-tree-{mask:02d}' and digest==record['model_sha256'],'Stale boundary worker')
 for file,expected in record['files'].items():require(sha(Path(file))==expected,'Changed boundary evidence: '+file)
 require(record['independent_review']['reviewer']=='root independent' and record['independent_review']['status']=='PASS scoped boundary receiver','Missing independent review')
 group=next(g for g in read(catalog_path)['groups'] if g['id']==worker.name)
 require(set(record['part_ids'])=={source_for_part(p) for p in group['parts']},'Source scope changed')
 for name in ('saved-model-audit.json','source-coverage/report.json','actual-materials/evidence.json'):
  require(read(inspection/name)['model_sha256']==digest,'Stale current worker audit')
 require(read(worker/'validation.json')['status']=='PASS' and read(inspection/'saved-model-audit.json')['status']=='PASS','Worker invalid')
 require(read(inspection/'source-coverage/report.json')['intersection_over_union']>.95,'Source coverage regression')
 texels=read(Path(record['texel_audit']));count={35:122,43:20,45:2,46:7}[mask]
 require(texels['model_sha256']==digest and texels['target_pixels']==count and texels['exact_source_matches']==count,'Inferred contour RGB coverage incomplete')
 proof=read(Path(record['preservation_proof']));require(proof['preserved'] and proof['geometry_and_material_outside_wood_unchanged'],'Outside scene changed')
 if mask==35:
  correction=read(inspection/'contour-rgb-restoration.json');require(correction['model_sha256']==digest and correction['previous_model_sha256']==proof['model_sha256'] and correction['other_texels_unchanged'] and correction['geometry_uv_unchanged'],'Invalid bounded RGB continuation')
 else:require(proof['model_sha256']==digest,'Stale preservation proof')
 require(sha(Path(proof['previous_worker'])/'model.blend')==proof['previous_model_sha256'],'Previous worker changed')
 return worker

def bind(out,mask,worker,review,review_text,catalog_path):
 worker=worker.resolve();review=review.resolve();path=out/'restart2-wood/selections'/f'tree-{mask}.json';require(not path.exists(),'Preserve earlier receipt');paths={worker/'model.blend',worker/'workspace.json',worker/'source-masks.json',worker/'validation.json'}
 original=out/f'restart2-wood/projected/assets/croisement02-tree-{mask}';proof=original/'inspection/boundary-preservation.json';paths.add(proof)
 for name in ('saved-model-audit.json','source-coverage/report.json','actual-materials/evidence.json'):paths.add(worker/'inspection'/name)
 for name in ('evidence.json','texel-audit.json','self-review.json','before-solid.png','after-solid.png','after-textured.png','source-comparison.png'):paths.add(review/name)
 if mask==35:paths.add(worker/'inspection/contour-rgb-restoration.json')
 manifest=read(worker/'source-masks.json');inventory=Path(manifest['mask_inventory']);paths.add(inventory)
 for row in read(inventory)['masks']:paths.add(Path(row['png']))
 write_json(path,dict(kind='scoped-inferred-wood-boundary',mask=mask,asset_id=worker.name,worker=str(worker),model_sha256=sha(worker/'model.blend'),part_ids=read(worker/'workspace.json')['part_ids'],approval='pending',whole_tree_status='HOLD: legacy crown refinement and texture completion',independent_review=dict(reviewer='root independent',status='PASS scoped boundary receiver',exact_review_text=review_text),texel_audit=str(review/'texel-audit.json'),preservation_proof=str(proof.resolve()),files={str(p.resolve()):sha(p) for p in sorted(paths)},limitations=['Inferred ownership labels remain; precise native RGB does not prove semantic bark identity.','Original branch seams, unknown rear wood and legacy canopy remain separate unfinished work.']))
 require(selected_workspace(out,mask,catalog_path)==worker,'Bound selection failed')
 return path

"""Strict scoped continuous wood selection, with exact inferred contour RGB and new basal inference."""
from pathlib import Path
from restart2_boundary_selections import read,require
from evidence_io import sha,write_json
from catalog_schema import source_for_part

def selected_workspace(out,mask,catalog_path):
 if mask not in (43,45,46):return None
 receipt=out/'restart2-wood/selections'/f'tree-{mask}-continuous-v1.json'
 if not receipt.exists():return None
 r=read(receipt);w=Path(r['worker']);digest=sha(w/'model.blend');require(r['kind']=='scoped-continuous-own-trace-wood' and r['model_sha256']==digest,'Stale continuous wood selection')
 require(r['approval']=='pending' and r['whole_tree_status'].startswith('HOLD:'),'No whole tree approval')
 for p,h in r['files'].items():require(sha(Path(p))==h,'Changed continuous wood evidence: '+p)
 require(r['independent_review']['status']=='PASS scoped continuous wood geometry','Missing root review')
 group=next(g for g in read(catalog_path)['groups'] if g['id']==w.name);require(set(r['part_ids'])=={source_for_part(p) for p in group['parts']},'Source ownership changed')
 for n in ('saved-model-audit.json','source-coverage/report.json','actual-materials/evidence.json'):require(read(w/'inspection'/n)['model_sha256']==digest,'Stale current verification')
 require(read(w/'validation.json')['status']=='PASS' and read(w/'inspection/saved-model-audit.json')['status']=='PASS','Invalid worker')
 require(read(w/'inspection/source-coverage/report.json')['intersection_over_union']>.95,'Native silhouette regression')
 texels=read(Path(r['texel_audit']));count={43:20,45:2,46:7}[mask];require(texels['model_sha256']==digest and texels['exact_source_matches']==texels['target_pixels']==count,'Incomplete contour RGB')
 proof=read(w/'inspection/boundary-preservation.json');require(proof['preserved'] and proof['geometry_and_material_outside_wood_unchanged'],'Outside wood changed')
 if mask==43:
  correction=read(w/'inspection/contour-rgb-restoration.json');require(correction['model_sha256']==digest and correction['previous_model_sha256']==proof['model_sha256'] and correction['other_texels_unchanged'] and correction['geometry_uv_unchanged'],'Invalid exact RGB continuation')
 else:require(proof['model_sha256']==digest,'Stale preservation')
 ground=read(Path(r['ground_evidence']));require(ground['model_sha256']==digest and ground['support_transforms_verified'] and ground['native_view0_elevation']==35,'Invalid ground proof')
 return w

def bind(out,mask,catalog_path):
 v={43:'branch-rgb-v3',45:'branch-projected-v3',46:'branch-projected-v4'}[mask];w=out/f'restart2-wood/{v}/assets/croisement02-tree-{mask}';review=out/f'restart2-wood/tree{mask}-branch-review-native-v4';ground=out/f'restart2-wood/tree{mask}-full-ground-native-v4';native=out/f'restart2-wood/tree{mask}-full-native-v{4 if mask==46 else 3}';receipt=out/'restart2-wood/selections'/f'tree-{mask}-continuous-v1.json';require(not receipt.exists(),'Preserve earlier selection')
 paths={w/'model.blend',w/'workspace.json',w/'source-masks.json',w/'validation.json',out/'restart2-wood/selections'/f'tree-{mask}.json'}
 for d in (review,ground,native):paths.update(p for p in d.iterdir() if p.suffix in ('.json','.png'))
 for n in ('saved-model-audit.json','source-coverage/report.json','actual-materials/evidence.json','actual-materials/sheet.png','boundary-preservation.json'):paths.add(w/'inspection'/n)
 if mask==43:paths.add(w/'inspection/contour-rgb-restoration.json')
 for sub in (f'tree{mask}-branch-sdf-v2',f'tree{mask}-branch-fitted-v{3 if mask==46 else 2}'):
  paths.add(out/'restart2-wood'/sub/'evidence.json');paths.add(out/'restart2-wood'/sub/'model.blend')
 inventory=Path(read(w/'source-masks.json')['mask_inventory']);paths.add(inventory)
 for row in read(inventory)['masks']:paths.add(Path(row['png']))
 write_json(receipt,dict(kind='scoped-continuous-own-trace-wood',mask=mask,worker=str(w),model_sha256=sha(w/'model.blend'),part_ids=read(w/'workspace.json')['part_ids'],approval='pending',whole_tree_status='HOLD: new crown composition and unknown bark texture completion',texel_audit=str(review/'texel-audit.json'),ground_evidence=str(ground/'evidence.json'),independent_review=dict(reviewer='root independent',status='PASS scoped continuous wood geometry',exact_review_text='Root personally inspected43/45/46 final native-first branch actual8, groundactual8, and full native source comparisons. Scoped continuous wood geometry PASS for b30f6fd8/cbd8d8a9/6964b640 pending final contourRGB audits/user. Continuous crotches/stems remove capped collar; new basal inference grounded, native flares plausible. Expose explicitly gray hidden bark/new complete basalshape, not preserved-oldroot claim.'),files={str(p.resolve()):sha(p) for p in sorted(paths)},limitations=['Complete transverse and basal volume newly inferred from own native trace; old root geometry not preserved.','Gray hidden bark remains unknown; exact contour RGB retains inferred ownership labels.','Current crown remains unfinished and no user approval is inherited.']))
 require(selected_workspace(out,mask,catalog_path)==w,'Selection validation failed')
 return receipt

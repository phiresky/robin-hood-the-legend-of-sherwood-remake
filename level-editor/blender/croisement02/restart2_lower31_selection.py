"""Guard the reviewed continuous lower31 base while foliage/native joint remains open."""
from pathlib import Path
from restart2_boundary_selections import read,require
from evidence_io import sha,write_json

def selected_workspace(out,mask,catalog_path):
 if mask!=31:return None
 path=out/'restart2-wood/selections/tree-31-v2.json'
 if not path.exists():path=out/'restart2-wood/selections/tree-31.json'
 if not path.exists():return None
 r=read(path);worker=Path(r['worker']);digest=sha(worker/'model.blend')
 require(r['kind']=='scoped-continuous-lower31' and digest==r['model_sha256'],'Stale lower31 selection')
 require(r['approval']=='pending' and r['native_joint_status'] in ('HOLD: exact local foliage/native joint proof required','PASS scoped own31 native foliage joint; adjacent plants excluded'),'Do not imply whole31 acceptance')
 for p,h in r['files'].items():require(sha(Path(p))==h,'Changed lower31 evidence')
 require(r['independent_review']['status']=='PASS scoped continuous lower construction','Missing root scoped review')
 for name in ('saved-model-audit.json','source-coverage/report.json','actual-materials/evidence.json','boundary-preservation.json'):
  require(read(worker/'inspection'/name)['model_sha256']==digest,'Stale lower31 verification')
 require(read(worker/'validation.json')['status']=='PASS' and read(worker/'inspection/saved-model-audit.json')['status']=='PASS','Invalid lower31')
 proof=read(worker/'inspection/boundary-preservation.json');require(proof['preserved'] and proof['geometry_and_material_outside_wood_unchanged'],'Outside31 scene changed')
 require(sha(Path(proof['previous_worker'])/'model.blend')==proof['previous_model_sha256'],'Frozen lower31 source changed')
 geometry=read(out/'restart2-wood/tree31-sdf-v5/evidence.json');require(geometry['full_geometry']['nonmanifold_edges']==0 and geometry['full_geometry']['degenerate_faces']==0 and geometry['upper_surface_to_old_distance_max']<.5,'Lower31 geometry invalid')
 if r['native_joint_status'].startswith('PASS'):
  joint=read(Path(r['joint_receipt']));require(joint['model_sha256']==digest and joint['independent_review']['status']=='PASS scoped own31 native foliage joint','Stale own31 joint receipt')
 return worker

def bind(out,catalog_path):
 worker=out/'restart2-wood/projected31-v5/assets/croisement02-tree-31';path=out/'restart2-wood/selections/tree-31.json';require(not path.exists(),'Preserve existing31 receipt');paths={worker/'model.blend',worker/'workspace.json',worker/'source-masks.json',worker/'validation.json',out/'restart2-wood/tree31-sdf-v5/evidence.json'}
 for name in ('saved-model-audit.json','source-coverage/report.json','actual-materials/evidence.json','boundary-preservation.json'):paths.add(worker/'inspection'/name)
 for directory,names in [('tree31-native-review-v5',['evidence.json','source-comparison.png','before-solid.png','after-solid.png']),('tree31-ground-actual-v5',['evidence.json','solid.png','textured.png'])]:
  for name in names:paths.add(out/'restart2-wood'/directory/name)
 write_json(path,dict(kind='scoped-continuous-lower31',worker=str(worker),model_sha256=sha(worker/'model.blend'),approval='pending',native_joint_status='HOLD: exact local foliage/native joint proof required',whole_tree_status='HOLD: legacy crown refinement and texture completion',independent_review=dict(reviewer='root independent',status='PASS scoped continuous lower construction',exact_review_text='Root viewed31v5 solid8, sourcecomparison, groundactual8 and fullactual8. Continuous lower construction scoped PASS1706f4d7, removes collar seams and rests plausibly. Native lower region heavily foliage obscured; need exact local foliage/native joint proof before whole31 readiness.'),files={str(p.resolve()):sha(p) for p in sorted(paths)},limitations=['Transverse lower volume inferred from native centreline/radii.','Upper retained surface differs at most0.429 world units following Boolean retessellation.','Local native foliage joint, crown cleanup and unknown wood texture remain open.']))
 require(selected_workspace(out,31,catalog_path)==worker,'Lower31 selection failed')
 return path

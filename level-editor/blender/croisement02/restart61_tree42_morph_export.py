"""Export approved crown poses against the current unchanged Tree42 source placement."""
import json,shutil,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart26_tree42_approved_morph_export as recipe
ROOT=recipe.ROOT
DEST=recipe.OUT/'restart61-tree42-approved-morph-v1'
CURRENT_MAP_SHA='be7e5b19e823691a2846846b295365d7283433402d02b5eebb4b3c07d6940520'
EXPECTED_PLACEMENT={'id':'croisement02-tree-42','name':'South Field Oak','transform':{'dx':794,'dy':961.3141073243531,'dz':0,'rot_deg':0},'assets':['croisement02-tree-42']}
EXPECTED_SOURCE={'model':'3d-assets/croisement02/croisement02-tree-42/model.glb','model_sha256':recipe.GLB_SHA,'descriptor':'3d-assets/croisement02/croisement02-tree-42/asset.json','descriptor_sha256':'37f3ba14ef07afb78961159bafc55f1eb87a79f505f91949f3a4bd333c2687dc','id':'croisement02-tree-42'}
original_map_deltas=recipe.map_deltas
correspondence=[]
def map_deltas(base_world,source_world,source_delta,linear,tolerance=1e-4):
 import numpy as np
 # World coordinates undergo float32 storage and matrix transforms on both sides.
 bound=max(tolerance,4*float(np.spacing(np.float32(np.max(np.abs(source_world))))))
 assert bound<=0.001
 mapped,error=original_map_deltas(base_world,source_world,source_delta,linear,bound)
 correspondence.append({"world_coordinate_float32_ulp":bound/4,"roundoff_bound":bound,"maximum_error":error,"vertices":len(base_world)})
 return mapped,error
recipe.map_deltas=map_deltas
original_verify=recipe.verify_inputs
recipe.DEST=DEST
recipe.MAP_SHA=CURRENT_MAP_SHA

def verify():
 original_verify()
 doc=json.loads(recipe.MAP.read_text())
 assert next(p for p in doc['placements']if p['id']=='croisement02-tree-42')==EXPECTED_PLACEMENT
 assert next(p for p in doc['assetSources']if p['id']=='croisement02-tree-42')==EXPECTED_SOURCE
 assert recipe.sha(ROOT/'level-editor/library'/EXPECTED_SOURCE['descriptor'])==EXPECTED_SOURCE['descriptor_sha256']
recipe.verify_inputs=verify

def main():
 verify()
 if '--capture'in sys.argv:
  sys.path.insert(0,str(ROOT/'level-editor/refinement'))
  from render_slots import acquire,release
  acquire(2)
  try:
   mem={k:int(v.split()[0])*1024 for k,v in (line.split(':',1)for line in Path('/proc/meminfo').read_text().splitlines())}
   assert mem['MemAvailable']>=6*1024**3 and shutil.disk_usage(recipe.OUT).free>=10*1024**3
   recipe.capture()
   (DEST/'semantic-rebase.json').write_text(json.dumps({'status':'CURRENT_TREE42_SOURCE_AND_PLACEMENT_PINNED','prior_whole_map_sha256':'616734426ba0bb56f145790cfa9013e05703ac8ab4df341dc27f4b0cc01e63ab','current_map_sha256':CURRENT_MAP_SHA,'placement':EXPECTED_PLACEMENT,'source':EXPECTED_SOURCE,'map_written':False,'correspondence_authority':'Augmentation additionally requires every static exported crown vertex to match the approved world-space basis within explicit float32 world-coordinate roundoff bound; no placement fitting permitted.'},indent=2)+'\n')
  finally:release()
 elif '--augment'in sys.argv:
  recipe.augment()
  report=json.loads((DEST/'export.json').read_text())
  report['limits'][-1]='Correspondence uses explicit float32 world-coordinate roundoff bound; see correspondence-roundoff.json. No fitting or delta changes.'
  (DEST/'export.json').write_text(json.dumps(report,indent=2)+'\n')
  (DEST/'correspondence-roundoff.json').write_text(json.dumps({'status':'APPROVED_BASIS_MATCH_WITH_FLOAT32_WORLD_ROUNDOFF','checks':correspondence,'geometry_fitted':False,'approved_deltas_changed':False},indent=2)+'\n')
 else:raise ValueError('Expected --capture or --augment')
if __name__=='__main__':main()

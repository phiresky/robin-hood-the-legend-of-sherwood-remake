"""Resume finite reopened checks and review renders after the placement receipt exists."""
from pathlib import Path
import json,subprocess,time,shutil
HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
WORK=ROOT/'work/croisement02-refinement/restart15-hiding-mounds/all-placements-v1'
def run():
 deadline=time.monotonic()+14400
 while not(WORK/'validation.json').exists():
  if time.monotonic()>deadline:raise TimeoutError('Placement construction did not finish within four hours')
  time.sleep(10)
 assert shutil.disk_usage(WORK).free>25*1024**3,'Disk reserve reached; review pipeline held'
 tasks=[('restart15_guard_closed_mounds.py','saved-native-guard.json'),('restart15_mound_shape_review.py','shape-review-v1/report.json'),('restart15_mound_contacts.py','contacts-v1/report.json')]
 for recipe,receipt in tasks:
  if(WORK/receipt).exists():continue
  subprocess.run(['/usr/bin/blender','--background','--python-exit-code','1','--python',str(HERE/recipe)],check=True)
  assert(WORK/receipt).exists(),receipt
  if recipe=='restart15_guard_closed_mounds.py':assert json.loads((WORK/receipt).read_text())['status']=='PASS'
 if not(WORK/'source-contexts-v1/report.json').exists():subprocess.run(['python3',str(HERE/'restart15_mound_source_contexts.py')],check=True)
 (WORK/'pipeline-complete.json').write_text(json.dumps({'status':'RENDERS_READY_FOR_VISUAL_REVIEW','scope':'No automatic geometry approval or publication.'},indent=2)+'\n')
if __name__=='__main__':run()

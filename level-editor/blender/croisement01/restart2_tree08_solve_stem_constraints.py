"""Bound the stem trial's active-set iterations; never promote a failed mesh."""
import json
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/active/croisement01/restart2'
recipes=Path(__file__).parent
prior=['tree08-main-stem-radial-fairing-v2-oriented','tree08-main-stem-radial-fairing-v3-crossing-buffer','tree08-main-stem-radial-fairing-v4-constraints']
result=R/'tree08-stem-constraint-solve-v1.json'
assert not result.exists()
rows=[]
for revision in range(5,11):
    name=f'tree08-main-stem-radial-fairing-v{revision}-constraints'
    folder=R/name
    print('BUILD',name,flush=True)
    with (R/f'{name}.log').open('w') as log:
        subprocess.run([sys.executable,str(recipes/'restart2_tree08_fair_main_stem_constraint_pass.py'),name,*prior],stdout=log,stderr=subprocess.STDOUT,check=True)
    with (folder/'guard-driver.log').open('w') as log:
        subprocess.run([sys.executable,str(recipes/'restart2_tree08_check_changed_candidate.py'),name],stdout=log,stderr=subprocess.STDOUT,check=True)
    guard=json.loads((folder/'intersection-guard.json').read_text())
    row=dict(packet=name,new_intersections=len(guard['new_intersections']),retained_intersections=len(guard['retained_intersections']))
    rows.append(row);print(json.dumps(row),flush=True)
    if not row['new_intersections'] and not row['retained_intersections']:
        with (folder/'native-coverage-guard.log').open('w') as log:
            subprocess.run([sys.executable,str(recipes/'restart2_tree08_trial_native_coverage.py'),name],stdout=log,stderr=subprocess.STDOUT,check=True)
        coverage=json.loads((folder/'native-coverage-guard.json').read_text())
        result.write_text(json.dumps(dict(status='STRICT_GEOMETRY_PASS_VISUAL_REVIEW_REQUIRED' if coverage['status']=='PASS_NATIVE_COVERAGE_AND_TOPOLOGY' else 'HOLD_NATIVE_COVERAGE',candidate=name,iterations=rows,coverage=coverage),indent=2)+'\n')
        break
    prior.append(name)
else:
    result.write_text(json.dumps(dict(status='HOLD_AFTER_BOUNDED_CONSTRAINT_TRIALS',iterations=rows),indent=2)+'\n')
print(result.read_text(),flush=True)

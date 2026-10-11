"""Freeze private v2 proposal with checkout-independent tests, retaining v1 unchanged."""
import difflib
import hashlib
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement/restart2/restart70-hall-visibility-private-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
old,new=OUT/'baseline',OUT/'mirror'
paths=sorted(str(p.relative_to(old)) for p in old.rglob('*.ts'))+[
 'level-editor/shared/src/source-cover-visibility.ts',
 'level-editor/app/src/source-cover-visibility.test.ts',
 'level-editor/app/src/source-cover-visibility.fixture.ts']
patch=[];pins={};outputs={}
for rel in paths:
 a,b=old/rel,new/rel
 if a.exists():
  assert sha(a)==sha(ROOT/rel),f'Shared baseline changed: {rel}'
  pins[rel]=sha(a)
 else:assert not (ROOT/rel).exists()
 patch.extend(difflib.unified_diff(a.read_text().splitlines(True) if a.exists() else [],b.read_text().splitlines(True),fromfile='a/'+rel if a.exists() else '/dev/null',tofile='b/'+rel))
 outputs[rel]=sha(b)
 if rel.endswith(('.test.ts','.fixture.ts')):
  added=''.join(x[1:] for x in difflib.unified_diff(a.read_text().splitlines(True) if a.exists() else [],b.read_text().splitlines(True)) if x.startswith('+') and not x.startswith('+++'))
  assert not any(s in added for s in ['restart69','restart70','level-editor/work/','level-editor/library/','readFile','process.cwd'])
(OUT/'runtime-proposal-v2.patch').write_text(''.join(patch))
(OUT/'baseline-pins-v2.json').write_text(json.dumps(pins,indent=2)+'\n')
r=json.loads((OUT/'handoff.json').read_text());r['patch_sha256']=sha(OUT/'runtime-proposal-v2.patch');r['shared_source_baselines']=pins;r['proposed_source_sha256']=outputs
r['version']=2;r['supersedes']='runtime-proposal.patch (preserved unchanged)'
r['tests']={'shared_passed':73,'shared_failed':0,'typescript_no_emit':True,'shared_log_sha256':sha(OUT/'test-results-v2.txt'),'outside_repository_cwd_passed':49,'outside_repository_log_sha256':sha(OUT/'outside-repo-tests-v2.txt'),'private_exact_york_passed':3,'private_exact_york_log_sha256':sha(OUT/'york-integration-results-v2.txt'),'private_exact_york_test_sha256':sha(OUT/'york-resource-integration.test.ts')}
r['test_isolation']='Shared tests generate source records, placement descriptors and 2x2 PNG resources entirely in memory. Exact York asset/pixel checks run separately in private stage, outside default app test discovery.'
(OUT/'handoff-v2.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps({'patch_sha256':r['patch_sha256'],'handoff_sha256':sha(OUT/'handoff-v2.json')}))

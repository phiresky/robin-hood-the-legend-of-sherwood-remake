"""Inspect a guarded tree texture bake without altering its approved geometry."""
import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).parent))
import render_candidate
from restart2_review_labels import labeled_sheet

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    p=argparse.ArgumentParser();p.add_argument('experiment',type=Path);p.add_argument('baked',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);e=a.experiment.resolve();b=a.baked.resolve();proof=json.loads((b/'reopened-preservation.json').read_text())
    if not all(proof[k] for k in ['geometry_unchanged','foreign_appearance_unchanged','physical_alpha_unchanged','known_foliage_rgba_unchanged','foliage_uv_and_ownership_unchanged']):raise ValueError('Incomplete bake preservation')
    if proof['reopened_preservation']!='PASS' or sha(b/'worker.blend')!=proof['candidate_model_sha256']:raise ValueError('Stale or failed reopened bake')
    out=b/'actual-review-v1';out.mkdir(exist_ok=False);(out/'inspection').mkdir();(out/'modified').mkdir();shutil.copy2(b/'worker.blend',out/'model.blend');shutil.copy2(e.parent/'approved-workspace/workspace.json',out/'workspace.json')
    packet=json.loads((e/'views.json').read_text());packet['source_blend']=str(out/'model.blend');(out/'modified/views.json').write_text(json.dumps(packet,indent=2)+'\n')
    sys.argv=['render','--',str(out)];render_candidate.main();labeled_sheet(out,'inspection/actual-materials/sheet.png')
    (out/'inspection/bake-binding.json').write_text(json.dumps(dict(approved_model_sha256=sha(e/'approved-model.blend'),baked_model_sha256=sha(b/'worker.blend'),review_model_sha256=sha(out/'model.blend'),preservation_report_sha256=sha(b/'reopened-preservation.json'),camera_manifest_sha256=sha(e/'views.json'),geometry_unchanged=True,texture_approval='pending'),indent=2)+'\n')
    print(out)
if __name__=='__main__':main()

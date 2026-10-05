"""Freeze reviewed source-gap geometry packets for the grouped gallery owner."""
import sys,json,hashlib
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from restart4_package_source_gaps import SPECS

def main(key):
 label,asset,_,_=SPECS[key];base=OUT/'restart4-source-gaps';w=base/'packaged-v1/assets'/asset;trial=base/label;h=sha(w/'model.blend')
 root=json.loads((w/'inspection/root-review.json').read_text());assert root['model_sha256']==h and root['status'].startswith('PASS scoped')
 for n in ['validation.json','inspection/saved-model-audit.json']:
  d=json.loads((w/n).read_text());assert d['status']=='PASS'
 notes=root['notes'];actual=w/'inspection/wide-contact/textured-sheet.png'
 write_json(w/'inspection/visual-review.json',dict(model_sha256=h,sheet_sha256=sha(actual),ready_for_geometry_review=True,notes=notes))
 write_json(w/'inspection/source-gap-disclosure.json',dict(model_sha256=h,notes=notes))
 paths=[w/n for n in ['model.blend','baseline.blend','workspace.json','source-masks.json','validation.json','input/views.json','input/solid.png','input/textured.png','modified/views.json','modified/solid.png','modified/textured.png','inspection/root-review.json','inspection/visual-review.json','inspection/source-gap-disclosure.json','inspection/saved-model-audit.json','inspection/saved-guard.json','inspection/refinement.json','inspection/prototype-preservation.json','inspection/wide-contact/textured-sheet.png','inspection/wide-contact/solid-sheet.png','inspection/wide-contact/source-comparison.png','inspection/wide-contact/contact-sheet.png','inspection/wide-contact/evidence.json']]
 if key=='shed':paths.append(trial/'wood-intersections.json')
 assert all(p.is_file() for p in paths)
 result=dict(asset_id=asset,worker=str(w),model_sha256=h,part_ids=['building-026','building-027'] if key=='stump' else ['building-138','building-139'],native_mask_indices=[107,108] if key=='stump' else [110,127],status='root reviewed; pending user geometry approval',texture_status='Approved58a3cbc5 parent fill retained; new inferred surfaces need fill after geometry approval.' if key=='shed' else 'Original fill retained with exact native cap overlay; no new synthesis approval implied.',actual_sheet=str(actual),solid_sheet=str(w/'inspection/wide-contact/solid-sheet.png'),source_trace=str(w/'inspection/wide-contact/source-comparison.png'),source_comparison=str(w/'inspection/source-comparison.png'),contact_sheet=str(w/'inspection/wide-contact/contact-sheet.png'),notes=notes,files={str(p):sha(p) for p in paths})
 out=base/(key+'-ready-candidate-v1.json');assert not out.exists();write_json(out,result);print(out)
if __name__=='__main__':main(sys.argv[sys.argv.index('--')+1])

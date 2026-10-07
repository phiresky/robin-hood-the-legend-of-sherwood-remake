"""Prepare a fresh ground-fill request preserving the approved atlas and domain exactly."""
import hashlib
import json
import shutil
from pathlib import Path
from PIL import Image
R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/croisement03-refinement/restart2/approved-hub-textures-v1/trio-ground'
OLD=B/'experiment'
OUT=B/'correction-v2'
PROMPT='The gray pixels are holes left by removed scenery, not objects. Replace each hole with continuous small-scale forest-floor texture that matches the immediately adjoining painted pixels: dense tiny olive-brown moss flecks, leaf litter, granular earth and mottled shadows. Carry the nearby texture frequency, contrast and warm/dark variation smoothly across every cutout boundary. Completely erase the visual shape of every former trunk: there must be no smooth vertical bands, upright strips, column-like shadows, flat green patches, sharp interior seams or tree-shaped regions. Do not shade these shapes as cylinders or preserve their outlines. Every formerly gray pixel needs the same fine irregular ground grain as its immediate surroundings, including the narrow lower strips. The enlarged target crop shows precisely where the fine nearby texture must continue; it is a locator, not a new output canvas. The two native floor examples show the required material detail. Keep all existing non-gray source pixels at their exact coordinates and colors. Do not add trunks, roots, branches, crowns, standing vegetation, large leaves, objects or terrain relief. Return the complete original 1408x960 atlas.'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False);e=OUT/'experiment';e.mkdir();prep=json.loads((OLD/'preparation.json').read_text())
 for name in ['input.png','mask.png','solid.png','views.json','approval.json']:
  assert sha(OLD/name)==prep['files'][name];shutil.copyfile(OLD/name,e/name)
 refs=json.loads((OLD/'auxiliary-references.json').read_text());crop=dict(left=908,top=0,width=294,height=212);image=Image.open(e/'input.png').convert('RGBA').crop((908,0,1202,212));target=e/'target-detail-2x.png';image.resize((588,424),Image.Resampling.NEAREST).save(target)
 refs['references'].insert(0,dict(source='input',file=str(target),sha256=sha(target),crop=crop,scale=2));(e/'auxiliary-references.json').write_text(json.dumps(refs,indent=2)+'\n')
 evidence=dict(status='PREPARED pending coordinator visual check; generation not run',original_experiment=str(OLD),experiment=str(e),approved_canvas_and_editable_domain_identical=True,editable_pixels=17447,outside_pixels_exact=True,no_new_geometry_or_ownership=True,input_dimensions=[1408,960],prompt_suffix=PROMPT,changes=['Add an exact 2x nearest-neighbor crop of the already approved input, showing local texture and every editable cutout.','Explicitly replace former trunk silhouettes with fine irregular floor grain matched to immediately adjacent source pixels; reject smooth vertical bands and cylinder shading.','Use a separate experiment and cache; preserve both earlier responses and their evidence.'],files={str(p):sha(p) for p in sorted(e.iterdir()) if p.is_file()},api_policy='Existing authorization persists. No generation until coordinator inspects revised input; no simultaneous request or old cache overwrite.')
 (OUT/'request-plan.json').write_text(json.dumps(evidence,indent=2)+'\n')
 (OUT/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Croisement03 corrected ground request</title><style>body{background:#252525;color:#eee;font:16px system-ui;margin:24px}img{image-rendering:pixelated}p{max-width:1000px}a{color:#9cf}</style><h1>Corrected ground fill request — not generated</h1><p>Same approved 1408×960 atlas and 17,447 editable pixels. Exact local target crop added as a supplementary image; both original floor examples retained. A new experiment keeps all prior evidence intact.</p><img src="experiment/target-detail-2x.png"><h2>New instructions</h2><p>'+PROMPT+'</p><a href="request-plan.json">Request plan and source hashes</a>')
 print(OUT)
if __name__=='__main__':main()

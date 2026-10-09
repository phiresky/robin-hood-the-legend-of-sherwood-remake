"""Prepare an unexecuted ground retry with an ordinary aligned region guide."""
import hashlib,json,shutil,shlex
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement03-refinement/restart2/approved-hub-textures-v1/trio-ground'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 old=B/'correction-v2/experiment';out=B/'correction-v3-region-guide';assert not out.exists();e=out/'experiment';e.mkdir(parents=True)
 for name in ['input.png','mask.png','solid.png','views.json','approval.json']:
  shutil.copyfile(old/name,e/name);assert sha(old/name)==sha(e/name)
 image=np.array(Image.open(e/'input.png').convert('RGBA'));mask=np.array(Image.open(e/'mask.png').convert('RGBA'))[:,:,3]==0
 assert image.shape==(960,1408,4) and mask.sum()==17447
 guide=image.copy();guide[mask]=[255,0,180,255]
 # Cyan distinguishes troublesome lower strips while keeping the exact editable union.
 yy,xx=np.indices(mask.shape);lower=mask&(yy>=110);guide[lower]=[0,220,255,255]
 assert np.array_equal(guide[~mask],image[~mask]);Image.fromarray(guide).save(e/'region-guide.png')
 Image.fromarray(guide).crop((908,0,1202,212)).resize((882,636),Image.Resampling.NEAREST).save(out/'guide-detail.png')
 refs=json.loads((old/'auxiliary-references.json').read_text());refs['references'].append(dict(source='region-guide',file=str(e/'region-guide.png'),sha256=sha(e/'region-guide.png'),role='MAGENTA and CYAN both identify the exact editable forest-floor holes. Cyan marks the troublesome lower trunk-shaped strips: these are flat ground, not trunks, roots or cylindrical surfaces. Extend fine irregular leaf-litter grain through every cyan pixel, matching the neighboring unmarked floor. All unmarked source pixels stay unchanged. These colors are diagnostic only and must never appear in the output.'))
 write(e/'auxiliary-references.json',refs)
 parent=json.loads((B/'correction-v2/request-plan.json').read_text());prompt=parent['prompt_suffix']+' The final ordinary reference image is an aligned colored region guide. Magenta marks upper ground holes; cyan marks lower ground holes whose previous fills incorrectly looked like smooth upright olive strips. Both colors mean flat ground to rebuild from adjacent granular forest-floor texture. In particular, destroy the vertical visual continuity of every cyan region using small irregular leaf-litter and earth details. Do not preserve cylinder shading or a smooth olive center. Do not copy guide colors. Return only the original full 1408x960 target sheet.'
 argv=['node','level-editor/pipeline/src/refinement/generate-textures.ts',str(e),'--generate','--provider','openrouter','--prompt-variant','short','--no-mask','--lighting-reference',str(e/'solid.png'),'--auxiliary-references',str(e/'auxiliary-references.json'),'--prompt-suffix',prompt]
 write(out/'request-plan.json',dict(status='PREPARED; coordinator input review required; no API call',original_experiment=str(old),experiment=str(e),dimensions=[1408,960],editable_pixels=17447,lower_guide_pixels=int(lower.sum()),original_input_mask_lighting_views_approval_exact=True,original_three_references_unchanged=True,guide_only_extra_reference=True,prompt_suffix=prompt,argv=argv,files={str(p):sha(p) for p in e.iterdir() if p.is_file()}))
 (out/'generate-after-coordinator-review.sh').write_text('#!/usr/bin/env bash\nset -euo pipefail\ncd '+shlex.quote(str(ROOT))+'\n'+shlex.join(argv)+'\n')
 (out/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>C3 proposed ground region guide</title><style>body{background:#222;color:#eee;font:16px system-ui;margin:24px}img{max-width:100%;image-rendering:pixelated}a{color:#acf}</style><h1>Proposed ground retry input — not generated</h1><p>Original input, dimensions, three references and all 17,447 editable pixels are unchanged. Added ordinary aligned reference marks upper holes magenta and troublesome lower strips cyan. It is not a provider mask; local pixel protection remains authoritative.</p><img src="guide-detail.png"><p><a href="request-plan.json">Request plan and frozen inputs</a></p>')
 print(out)
if __name__=='__main__':main()

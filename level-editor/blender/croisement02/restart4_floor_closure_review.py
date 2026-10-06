"""Build a private review page for the remaining floor scope and phase evidence."""
import json,hashlib,html
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart4-floor-closure-input-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=json.loads((D/'proposal.json').read_text());base=Image.open(OUT/'restart4-remaining-floor-bake-v1/composite.png');source=Image.open(OUT/'source-states/covered.png');proposal=Image.open(D/'proposed-appearance.png');pts=[v for v in p['phase_pixels'] if v['native_return']];assert len(pts)==13
 sheet=Image.new('RGB',(1000,13*85),'#292929');draw=ImageDraw.Draw(sheet)
 for i,v in enumerate(pts):
  x,y=v['x'],v['y'];draw.text((5,i*85+8),f'Native [{x},{y}]',fill='white')
  for j,(im,name) in enumerate([(source,'Original background'),(base,'Approved floor'),(proposal,'Proposed exact return')]):
   pic=im.crop((x-3,y-3,x+4,y+4)).convert('RGB').resize((63,63),Image.Resampling.NEAREST);sheet.paste(pic,(200+j*255,i*85+18));draw.text((200+j*255,i*85+2),name,fill='white');draw.rectangle((200+j*255+27,i*85+18+27,200+j*255+35,i*85+18+35),outline='cyan')
 sheet.save(D/'native13-close.png')
 current=OUT/'restart2-textures/batch10-linked-static-review-v1/orbit-512'
 assets=[(current/'view-0.png','Current linked scene — original game camera'),(D/'region-guide.png','Exact proposed scope: orange5917 inferred, cyan13 native'),(D/'native13-close.png','All13 exact native returns'),*( (D/f'source-reuse-contexts-{i}.png',f'Source/current-ground/reuse contexts {i+1}/5') for i in range(5)),(current/'view-2.png','Current oblique2 — separate gray attribution pending'),(current/'view-4.png','Current oblique4 — separate gray attribution pending'),(OUT/'restart2-ground-completion/preparation-v1/reference-review.png','Four native ground examples used by existing raw response')]
 sections=''.join(f'<h2>{html.escape(label)}</h2><a href="/@fs{path}"><img src="/@fs{path}"></a>' for path,label in assets)
 body=f'''<!doctype html><meta charset="utf-8"><title>Croisement02 remaining floor input — private review</title><style>body{{background:#181b20;color:#eee;font:16px system-ui;max-width:1250px;margin:28px auto}}img{{max-width:100%;height:auto}}h2{{font-size:19px}}p{{line-height:1.5}}a{{color:#aad5ff}}</style><h1>Remaining floor input — private root review</h1><p>Proposed5917 inferred underlying-floor texels plus13 exact native returns on approved76bba. No model bake or synthesis. This is input-scope review only.</p><p>Native first-hit audit finds487 exposed and5443 covered centers. Exposed source wood and silhouette mismatches remain separate foreground geometry obligations; filling floor does not claim their completion. All772238 known pixels and every outside pixel remain exact.</p><p>Elevated hiding-Pc artwork and animated Arbre03 remain separate. Legacy569880 bank-underlay exclusion is not included; the two oblique gray patches await independent attribution.</p><p><a href="proposal.json">Exact proposal/hashes</a> · <a href="validation.json">Preservation validation</a></p>{sections}'''
 (D/'index.html').write_text(body)
 evidence=dict(status='private-root-review',proposal_sha256=sha(D/'proposal.json'),images=[dict(path=str(path),label=label,sha256=sha(path)) for path,label in assets],self_review='Viewed all five grouped source/current/reuse sheets. Underlying grass/soil plausible; softer reused patches disclosed. Exposed foreground deficits are not solved by this floor proposal.',native_returns=13,api_calls=0,model_writes=0)
 (D/'review-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n');print('review ready',sha(D/'proposal.json'))
if __name__=='__main__':main()

"""Group bounded residual flat-ground diagnostics without changing source ownership."""
import json,hashlib
from pathlib import Path
from collections import defaultdict
import numpy as np
from PIL import Image,ImageDraw
from scipy import ndimage
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart3-remaining-floor-audit-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def mask(p):return np.array(Image.open(p).convert('L'))>0
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 r=json.loads((D/'report.json').read_text());candidate=mask(D/'connected-gray-candidate-union.png');held=np.zeros_like(candidate);held[242:255,1365:1371]=candidate[242:255,1365:1371];assert held.sum()==49
 proposed=candidate&~held;assert proposed.sum()==14876
 Image.fromarray(held.astype('uint8')*255).save(D/'unassigned49-hold.png');Image.fromarray(proposed.astype('uint8')*255).save(D/'proposed-inferred-floor-union.png')
 names={'130':'Northwest foliage source130','97':'Southeast wall/fence source97','43':'Mixed tree43/46 root and vegetation context','108':'Logging wood/stump source108','129':'Western foliage source129','16':'Tree16 trunk/root context','86':'Native vegetation source86','132':'Northeast foliage source132','124':'Haystack source124','128':'Southern foliage source128 beyond pending2441','46':'Tree46 root/vegetation context','61':'Native shrub/wood source61','91':'Native vegetation source91','44':'Seven native wood44 edge pixels','unassigned-fringe':'Unassigned49 — HOLD'}
 groups=defaultdict(lambda:dict(pixels=0,components=[]))
 for c in r['components']:
  key=str(c['source_masks'][0]['index'])if c['source_masks']else 'unassigned-fringe';groups[key]['pixels']+=c['atlas_pixels'];groups[key]['components'].append(c['component'])
 rows=[dict(dominant_source_context=k,name=names[k],**v,scope='HOLD: source role not established'if k=='unassigned-fringe'else 'Proposed inferred underlying floor only; foreground ownership unchanged')for k,v in groups.items()]
 source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));over=source.copy();over[proposed,:3]=(over[proposed,:3]*.3+np.array([235,140,35])*.7).astype('uint8');over[held,:3]=[240,50,90];Image.fromarray(over).save(D/'source-completion-overlay.png')
 im=Image.new('RGB',(1280,960),'#303030');draw=ImageDraw.Draw(im)
 for i,(family,view)in enumerate([(f,v)for f in ['log-trap','south-cart','north-cart']for v in ['native','oblique']]):
  pic=Image.open(D/f'{family}-{view}-classified.png').convert('RGB');pic.thumbnail((420,420));x=i%3*426;y=i//3*470;im.paste(pic,(x,y+28));draw.text((x+4,y+7),family+' '+view,fill='white')
 draw.text((5,940),'Orange=new gray floor; blue=pending5419; purple=pending2441. No known/relief hits.',fill='white');im.save(D/'classified-views.png')
 inputs={'known':OUT/'restart2-ground-completion/preparation-v1/known.png','relief':OUT/'restart2-ground-completion/preparation-v1/separate_relief.png','state8201':OUT/'restart2-state/underlay-aggregate-audit-v3/combined-candidate-domain.png','fence5419':OUT/'restart3-initial-fence/floor-proposal-v2/inferred-hidden-floor.png','tree45-2441':OUT/'restart3-initial-fence/tree45-floor-proposal-v1/inferred-floor-domain.png'}
 guards={k:dict(path=str(p),sha256=sha(p),overlap=int((proposed&mask(p)).sum()))for k,p in inputs.items()};assert all(g['overlap']==0 for g in guards.values())
 report=dict(status='Bounded source-context completion proposal; root review required, no API or model change',diagnostic_report_sha256=sha(D/'report.json'),diagnostic_connected_components=len(r['components']),diagnostic_union_pixels=14925,proposed_inferred_floor_pixels=14876,held_unassigned_pixels=49,proposed_mask=str(D/'proposed-inferred-floor-union.png'),proposed_mask_sha256=sha(D/'proposed-inferred-floor-union.png'),held_mask_sha256=sha(D/'unassigned49-hold.png'),known_pixels_preserved=772189,native_returns=0,source_ownership_transfer=False,guards=guards,groups=rows,justification=['Each diagnostic render sample maps to exact gray atlas texels at flatZ0; none maps to known ground, separate relief or an already-filled state8201 texel.','The residual completion set expands visible seeds to entire connected gray patches so small disconnected edges remain tracked together, not forgotten.','Source crops show retained foreground trees, vegetation, logging wood, wall/fence and hay above these floor domains. Their source artwork stays on its original receiver.','These local state contexts omit some static neighbors; their visible gray is not proof of missing object geometry. Inferred underlying floor remains a separate appearance task.'],limitations=['Camera/atlas agreement is a read-only attribution check, not a newly rendered material-ID pass.','Dominant source context labels summarize components with overlapping source masks, not exclusive pixel ownership.','49 unassigned pixels near[1365,242] require exact initial/source role classification before inclusion.','This finite set covers the six inspected views; it is not an arbitrary-camera whole-map completeness claim.','Existing floor reuse suitability has not yet been assessed for this broader set. No synthesis or local atlas/model bake is authorized by this proposal.'])
 write(D/'completion-proposal.json',report);print(json.dumps({'proposed':14876,'held':49,'groups':rows},indent=2))
if __name__=='__main__':main()

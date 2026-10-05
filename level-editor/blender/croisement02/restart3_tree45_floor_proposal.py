"""Propose an exact inferred under-canopy floor domain and bounded existing-fill reuse."""
import json,hashlib,shutil
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart3-initial-fence/tree45-floor-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 assert shutil.disk_usage(OUT).free>25*1024**3
 D.mkdir(exist_ok=False)
 prior=OUT/'restart3-initial-fence/floor-fill-v1';basepath=prior/'bake-v1/composite.png';rawpath=prior/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png';model=prior/'bake-v1/model.blend';auditpath=OUT/'restart3-initial-fence/tree45-reservation-context-v1/first-hit.json';audit=json.loads(auditpath.read_text());assert audit['first_hits']=={'croisement02-tree-45':2441}
 base=np.array(Image.open(basepath).convert('RGBA'));raw=np.array(Image.open(rawpath).convert('RGBA'));domain=np.zeros(base.shape[:2],bool)
 for row in audit['samples']:x,y=row['pixel'];domain[y,x]=True
 knownpath=OUT/'restart2-ground-completion/preparation-v1/known.png';known=np.array(Image.open(knownpath).convert('L'))>0
 statepath=OUT/'restart2-state/underlay-aggregate-audit-v3/combined-candidate-domain.png';state=np.array(Image.open(statepath).convert('L'))>0
 fencepath=OUT/'restart3-initial-fence/floor-proposal-v2/inferred-hidden-floor.png';fence=np.array(Image.open(fencepath).convert('L'))>0
 assert domain.sum()==2441 and known.sum()==772189 and state.sum()==8201 and fence.sum()==5419
 assert not(domain&(known|state|fence)).any()
 trap14=[(329,366),(332,370),(334,372),(294,376),(337,376),(292,377),(290,378),(286,380),(284,381),(282,382),(280,383),(276,385),(274,386),(272,387)]
 assert all(not domain[y,x]for x,y in trap14)
 composite=base.copy();composite[domain,:3]=raw[domain,:3];assert np.array_equal(composite[~domain],base[~domain]) and np.array_equal(composite[:,:,3],base[:,:,3])
 Image.fromarray(base).save(D/'input.png');Image.fromarray(domain.astype('uint8')*255).save(D/'inferred-floor-domain.png');mask=np.full_like(base,255);mask[domain,3]=0;Image.fromarray(mask).save(D/'mask.png')
 guide=base.copy();guide[domain,:3]=[235,140,35];Image.fromarray(guide).save(D/'region-guide.png');Image.fromarray(composite).save(D/'proposed-reuse-preview.png')
 sourcepath=OUT/'animation-references/composite-frame-0.png';source=np.array(Image.open(sourcepath).convert('RGBA'));over=source.copy();over[domain,:3]=(over[domain,:3]*.35+np.array([235,140,35])*.65).astype('uint8')
 def sheet(name,panels):
  im=Image.new('RGB',(456*len(panels),488),'#303030');draw=ImageDraw.Draw(im)
  for i,(title,pixels)in enumerate(panels):im.paste(Image.fromarray(pixels).convert('RGB').crop((1018,811,1170,963)).resize((456,456),Image.Resampling.NEAREST),(i*456,32));draw.text((i*456+4,8),title,fill='white')
  im.save(D/name)
 sheet('source-role-close.png',[('Native foliage remains tree-owned',source),('Orange: proposed inferred floor underneath',over)])
 sheet('reuse-close.png',[('Current floor; tree omitted',base),('Existing raw response',raw),('Proposed bounded reuse; no model change',composite)])
 refs=OUT/'restart3-initial-fence/floor-proposal-v2/inputs-v1';request=json.loads((refs/'request.json').read_text())
 record=dict(status='PROPOSAL ONLY; no API or model mutation',scope='2441 inferred underlying floor pixels; native mask128 foliage stays owned by tree45',ground_model=str(model),ground_model_sha256=sha(model),ground_geometry_unchanged=True,base_appearance_status='Current5419floor candidate4e2c98fb has root PASS and remains pending user appearance approval; prior14 correction user approved',editable_pixels=2441,native_returns=0,native_tree_coverage=2441,known_pixels_protected=772189,outside_pixels_preserved=int((~domain).sum()),alpha_exact=True,prior5419_floor_preserved=True,prior14_pixels_preserved=True,state_underlay_domain=str(statepath),state_underlay_sha256=sha(statepath),state_underlay_pixels=8201,state_domain_overlap=0,fence5419_domain_overlap=0,physical_audit_sha256=sha(auditpath),model_saved=False,api_called=False,source_sha256=sha(sourcepath),input_sha256=sha(D/'input.png'),mask_sha256=sha(D/'mask.png'),region_guide_sha256=sha(D/'region-guide.png'),proposed_reuse_preview_sha256=sha(D/'proposed-reuse-preview.png'),existing_raw_response=str(rawpath),existing_raw_sha256=sha(rawpath),reuse_status='Candidate reuse suitability requires visual review; prior API approval did not authorize these2441 pixels',references=request['references'],reference_sheet=str(refs/'references.png'),hold='Approve this exact inferred floor region and existing raw material reuse before any local model bake. No new synthesis is requested or authorized by this proposal. Baked appearance remains separately reviewable.',initial_only=True,applied_terminal_separate=True)
 write(D/'proposal.json',record)
 write(D/'validation.json',dict(status='PASS for proposal review',proposal_sha256=sha(D/'proposal.json'),domain_pixels=2441,known_overlap=0,all_outside_pixels_exact=True,alpha_exact=True,state8201_disjoint=True,fence5419_disjoint=True,prior14_disjoint=True,source_ownership_transfer=False,model_saved=False,api_called=False))
 print(D)
if __name__=='__main__':main()

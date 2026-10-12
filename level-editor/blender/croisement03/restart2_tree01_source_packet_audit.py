"""Revalidate frozen Tree01 evidence and distinguish native authority from unassigned proposals."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';F=B/'tree01-native-transition-saved-v2';O=B/'tree01-source-packet-audit-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def main():
 assert not O.exists();O.mkdir();model=F/'worker.blend';digest=sha(model);receipts={};pins={str(model):digest}
 for name in ['receipt.json','native-source-guard.json','static-source-guard.json','all-phase-native-coverage-v1/receipt.json','phase-geometry-proof-v1/receipt.json','neighbor-review-v3/firsthits.json','crown-context-guard-v2.json','context-review-v1/receipt.json']:
  p=F/name;d=read(p);pins[str(p)]=sha(p);binding=d.get('model_sha256') or d.get('pins',{}).get(str(model));
  if binding is not None:assert binding==digest,(name,binding,digest)
  for key in ['input_hashes','pins','source_hashes']:
   for filename,expected in d.get(key,{}).items():
    path=Path(filename);assert path.exists() and sha(path)==expected,(name,filename);pins[filename]=expected
  for image,expected in d.get('images',{}).items():assert sha(p.parent/image)==expected,(name,image)
  receipts[name]={'sha256':sha(p),'status':d.get('status'),'model_binding':binding}
 for kind in ['native','static']:
  p=B/'tree01-native-transition-cpu-v2'/f'cells-{kind}.json';d=read(p);path=Path(d['source_frame']);assert sha(path)==d['source_frame_sha256'];pins[str(path)]=sha(path)
 native=np.zeros((175,175),bool);phases=[]
 for i in range(14):
  p=B/'tree01-native-fragment-v1'/f'{i:03}.png';a=np.array(Image.open(p).convert('RGBA'));native|=a[:,:,3]>0;phases.append(a[:,:,3]>0);pins[str(p)]=sha(p)
 static_path=B/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png';static_rgba=np.array(Image.open(static_path).convert('RGBA'));static=static_rgba[:,:,3]>0;original_path=B.parent/'baseline/covered.png';original=np.array(Image.open(original_path).convert('RGBA'))[:175,:175];assert np.array_equal(static_rgba[static],original[static]);assert not np.any(static&native)
 old=np.array(Image.open(B/'tree01-bark-seed-proposal-v1/proposed-visible-bark.png'))>0;upper=np.array(Image.open(B/'tree01-upper-bark-proposal-v1/proposed-upper-bark.png'))>0;assert not np.any(old&upper);bark=old|upper;canopy_conflict=bark[:175,:175]&(native|static)
 failures={tuple(f['source'])for row in read(B/'tree01-upper-bark-proposal-v1/cpu-firsthits.json')['checks']for f in row['failures']};eligible=upper.copy()
 for x,y in failures:eligible[y,x]=False
 assert int(eligible.sum())==98
 role=np.zeros((175,175,3),np.uint8);role[:]=[45,45,45];role[native]=[40,170,255];role[static]=[245,190,50];role[bark[:175,:175]]=[255,90,180];role[canopy_conflict]=[255,255,255];Image.fromarray(role).resize((700,700),Image.Resampling.NEAREST).save(O/'role-map.png');Image.fromarray(eligible.astype('uint8')*255).save(O/'upper-hit-consistent-unassigned.png')
 summary={'status':'PASS_FROZEN_EVIDENCE_UNASSIGNED_ROLE_ANALYSIS','model_sha256':digest,'pins':pins,'receipts':receipts,'roles':{'native_authority_union':int(native.sum()),'static_exact_source_proposal':int(static.sum()),'native_static_overlap':int((native&static).sum()),'old_bark_spines':int(old.sum()),'new_upper_spines':int(upper.sum()),'upper_hit_consistent_unassigned':98,'upper_miss_unassigned':len(failures),'bark_overlapping_observed_or_proposed_foliage':int(canopy_conflict.sum()),'per_phase_bark_native_overlap':[int((bark[:175,:175]&p).sum())for p in phases]},'limits':['Colored role map is an evidence index, not a final ownership mask. Gray means unclassified, never proven background.','Static6031 and bark316 are source proposals; native observed animation retains foreground priority.','98 upper receiver-consistent samples remain unassigned until source review; seven misses are not silently moved to nearby receivers.','Saved geometry/source coverage proof does not establish original native runtime clock or final single-owner integration.']};(O/'receipt.json').write_text(json.dumps(summary,indent=2)+'\n');print(summary['roles'])
if __name__=='__main__':main()

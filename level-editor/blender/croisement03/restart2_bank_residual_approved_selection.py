"""Freeze explicitly authorized private residual stone trial; no source assignment."""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';W=B/'restart2'
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);out=p.parse_args().output;assert not out.exists();pins={}
 def pin(path):pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return path
 def read(path):return json.loads(pin(path).read_text())
 evidence=read(W/'bank-residual-source-context-v3/receipt.json');review=read(W/'bank-residual-source-context-v3/self-review.json');priorpath=pin(W/'bank-residual-source-context-v2/residual-review-classes.png');old=np.array(Image.open(priorpath))==1;assert int(old.sum())==378;sourcepath=pin(B/'baseline/covered.png');assert pins[str(sourcepath)]==evidence['pins'][str(sourcepath)];src=np.array(Image.open(sourcepath).convert('RGBA'));selected=np.zeros(old.shape,bool);held=np.zeros(old.shape,bool);rows=[]
 accepted={'central-upper-window-a':42,'central-upper-window-b':39,'central-upper-window-c':53,'south-high-window':88}
 for r in evidence['regions']:
  mask=np.zeros(old.shape,bool)
  for x,y in r['exact_proposed_pixels_xy']:mask[y,x]=True
  assert int(mask.sum())==r['original_proposed_pixels'] and np.all(old[mask]);chosen=r['id']in accepted
  if chosen:assert int(mask.sum())==accepted[r['id']];selected|=mask
  else:held|=mask
  pin(W/'bank-residual-source-context-v3'/r['evidence']);rows.append({'id':r['id'],'status':'AUTHORIZED_PRIVATE_PROJECTION_TRIAL'if chosen else'HOLD_NOT_SELECTED','count':int(mask.sum()),'source_pixels_xy':r['exact_proposed_pixels_xy'],'source_evidence':r['evidence']})
 assert int(selected.sum())==222 and int(held.sum())==156 and not np.any(selected&held) and np.array_equal(selected|held,old)
 inventory=read(B/'source-inventory.json');phases=[]
 for animation_index in [4,7]:
  animation=next(a for a in inventory['animations']if a['index']==animation_index)
  assert len(animation['frames'])==14
  for i,frame in enumerate(animation['frames']):
   rawpath=pin(Path(frame['source']));normpath=pin(Path(frame['image']));assert pins[str(rawpath)]==frame['sha256'];raw=np.array(Image.open(rawpath).convert('RGBA'));normal=np.array(Image.open(normpath).convert('RGBA'));key=np.all(raw[:,:,:3]==[0,251,0],axis=2);raw[key]=0;assert np.array_equal(raw,normal);x,y,w,h=frame['bbox'];assert (w,h)==(normal.shape[1],normal.shape[0]);domain=selected[y:y+h,x:x+w];overlap=np.argwhere(domain&(normal[:,:,3]>0));assert len(overlap)==0,(animation_index,i,overlap.tolist());phases.append({'animation':animation_index,'phase':i,'native_box_xywh':frame['bbox'],'raw_sha256':pins[str(rawpath)],'normalized_sha256':pins[str(normpath)],'full_rgba_key_conversion_exact':True,'selected_alpha_overlap':0})
 out.mkdir();maskpath=out/'exact222-private-trial-mask.png';rgba=np.zeros_like(src);rgba[selected]=src[selected];Image.fromarray(selected.astype('uint8')*255).save(maskpath);Image.fromarray(rgba).save(out/'exact222-source-rgba.png');assert np.array_equal(np.array(Image.open(out/'exact222-source-rgba.png'))[selected],src[selected]);assert np.count_nonzero(rgba[:,:,3])==222
 coordinates=[[int(x),int(y)]for y,x in np.argwhere(selected)];report={'status':'AUTHORIZED222_PRIVATE_TRIAL_SOURCE_SELECTION_NOT_ASSIGNMENT','pins':pins,'authorization':{'delivered_by':'/root/crossings03_refinement','instruction':'Root now explicitly permits private central-upper a/b/c + south-high42+39+53+88=222 trial, leaves east114/south-right42 HOLD. Prepare NEW immutable bank-residual-approved-selection-v1 exact222mask/provenance linkedv3+rootmessage, fullnative exclusion proof and previous378exact remains. No projection/Blender.','scope':'Private source-projection trial after parent savedreceiver checks only. No geometry/material/source assignment or final acceptance by this selection helper.'},'source_sha256':pins[str(sourcepath)],'original_proposal_count':378,'selected_count':222,'held_count':156,'regions':rows,'exact_selected_pixels_xy':coordinates,'selected_mask_sha256':hashlib.sha256(maskpath.read_bytes()).hexdigest(),'selected_rgba_sha256':hashlib.sha256((out/'exact222-source-rgba.png').read_bytes()).hexdigest(),'full_native_phase_proof':phases,'original378_mask_unchanged':hashlib.sha256(priorpath.read_bytes()).hexdigest()==pins[str(priorpath)],'limits':['Previous source qualifications remain visible in v3 self-review; root authorizes a bounded private trial, not wholesale ownership acceptance.','East114 and south-right42 remain HOLD and excluded exactly.','No additional pixels created, nearest-color expansion, source subtraction, model edits or projection performed.','Await joined source review; no further tiny source expansions.']};assert report['original378_mask_unchanged'];(out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'selected':222,'held':156,'phase_checks':len(phases),'mask_sha256':report['selected_mask_sha256'],'rgba_sha256':report['selected_rgba_sha256']}))
if __name__=='__main__':main()

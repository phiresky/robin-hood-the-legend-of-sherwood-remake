"""Compare capped physical trial with its immutable parent and source phases."""
from pathlib import Path
import json,hashlib,shutil
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import distance_transform_edt
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';OLD=BASE/'tree42-motion-v4';NEW=BASE/'tree42-motion-v5';DEST=NEW/'comparison';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def guard(n=4*2**20):
 assert shutil.disk_usage(BASE).free-n>=10*2**30
 assert sum(p.stat().st_size for p in NEW.rglob('*')if p.is_file())+n<=256*2**20

def main():
 guard();DEST.mkdir(exist_ok=False);old=json.loads((OLD/'native-phases-v1/report.json').read_text());new=json.loads((NEW/'native-phases-v1/report.json').read_text());source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];rows=[];images={};distances={}
 for phase,f in enumerate(source['frames']):
  x,y,w,h=f['bbox'];n=Image.new('RGBA',(342,288));n.paste(Image.open(f['path']),(x-616,y-688));expected=np.array(n)[:,:,3]>=128;a=np.array(Image.open(OLD/f'native-phases-v1/phase-{phase:02}.png'));b=np.array(Image.open(NEW/f'native-phases-v1/phase-{phase:02}.png'));da=distance_transform_edt(a[:,:,3]<128);db=distance_transform_edt(b[:,:,3]<128);ga=expected&(da>2);gb=expected&(db>2);coords=lambda q:[[int(x+616),int(y+688)]for y,x in zip(*np.nonzero(q))];rows.append({'phase':phase,'raw_missing_before':int((expected&(a[:,:,3]<128)).sum()),'raw_missing_after':int((expected&(b[:,:,3]<128)).sum()),'distant_before':int(ga.sum()),'distant_after':int(gb.sum()),'distant_resolved':coords(ga&~gb),'distant_added':coords(gb&~ga),'max_alpha_distance_before':float(da[expected].max()),'max_alpha_distance_after':float(db[expected].max())})
  if phase in(0,7):images[phase]=(n,Image.fromarray(a),Image.fromarray(b));distances[phase]=(da,db)
 native0=np.array_equal(np.array(images[0][1]),np.array(images[0][2]));prior_gaps=json.loads((OLD/'native-phases-v1/phase7-distant-support-gaps.json').read_text())['pixels'];points=[p['native_pixel']for p in prior_gaps]+[[862,775]];classified=[];sheets=[]
 for i,(sx,sy)in enumerate(points):
  x,y=sx-616,sy-688;before=float(distances[7][0][y,x]);after=float(distances[7][1][y,x]);status='improved'if after<before else 'worse'if after>before else 'unchanged';classified.append({'native_pixel':[sx,sy],'before':before,'after':after,'comparison':status,'within_two_after':after<=2})
  if i%6==0:sheets.append(Image.new('RGB',(504,6*126+34),'#292929'));ImageDraw.Draw(sheets[-1]).text((8,8),'Own source7 | v4 actual7 | v5 actual7',fill='white')
  sheet=sheets[-1];d=ImageDraw.Draw(sheet);top=34+i%6*126;d.text((8,top),f'({sx},{sy}) {before:.2f}px -> {after:.2f}px: {status}',fill='white')
  for j,im in enumerate(images[7]):
   crop=im.crop((x-9,y-9,x+10,y+10)).resize((95,95),Image.Resampling.NEAREST);left=j*168+28;sheet.paste(crop,(left,top+22),crop);d.rectangle((left+45,top+67,left+49,top+71),outline='#ff5555')
 for i,im in enumerate(sheets):guard();im.save(DEST/f'original-gaps-{i+1}.png')
 a=json.loads((OLD/'review-v3/report.json').read_text());b=json.loads((NEW/'review-v3/report.json').read_text());assert a['fixed_cameras']==b['fixed_cameras'];full=Image.new('RGB',(1536,1616),'#ddd');full.paste(Image.open(OLD/'review-v3/actual-eight.png'),(0,0));full.paste(Image.open(NEW/'review-v3/actual-eight.png'),(0,808));guard();full.save(DEST/'v4-v5-eight.png');assert sha(OLD/'prototype.blend')=='ed90774d18790d35b23b2d20941c609b2420004ee4b4a1b293872ba934150376';report={'status':'MEASURED_FOR_AUTHOR_AND_ROOT_REVIEW_NOT_ACCEPTANCE','parent_model_sha256':sha(OLD/'prototype.blend'),'trial_model_sha256':sha(NEW/'prototype.blend'),'native0_rgba_exact':native0,'native0_max_rgba_difference':int(np.abs(np.array(images[0][1]).astype(int)-np.array(images[0][2]).astype(int)).max()),'loop_render_exact':b['loop_exact'],'fixed_eight_cameras_exact':True,'all_phase_coverage':rows,'original21_and_solid_target':classified,'solid_misses_before':a['per_phase_solid_support'],'solid_misses_after':b['per_phase_solid_support'],'limitations':['Raw checker-center coverage and distance to complete rendered alpha remain distinct from observed-front ownership.','No neighboring assets were loaded or changed for this scoped physical trial.','CPU feature improvement is not substituted for the actual phase/support measurements here.'],'resources':[json.loads((NEW/f'{n}-resources.json').read_text())for n in('build','native','eight')]};guard();(DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('phase7',rows[7]);print('native0 exact',native0);print('old21',classified[:21])
if __name__=='__main__':main()

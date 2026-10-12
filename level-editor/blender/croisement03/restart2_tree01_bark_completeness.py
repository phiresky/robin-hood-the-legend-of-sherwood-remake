"""Conservative manual bark proposals checked against full shared native alpha."""
import json,hashlib,argparse
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03';W=B/'restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);out=p.parse_args().output;assert not (out/'receipt.json').exists();out.mkdir(exist_ok=True);pins={}
 def read(path):pins[str(path)]=sha(path);return json.loads(path.read_text())
 def im(path):pins[str(path)]=sha(path);return np.array(Image.open(path).convert('RGBA'))
 review=read(W/'tree01-root-source-review-oct12-v1/review.json');project=read(W/'tree01-bark-seed-projection-v1/receipt.json');exclusions=read(W/'tree01-role-exclusions-v1/receipt.json')
 for receipt in [review,project,exclusions]:
  entries=receipt.get('pins',{});entries=entries.items()if isinstance(entries,dict)else[(r['path'],r['sha256'])for r in entries]
  for name,digest in entries:assert sha(Path(name))==digest,name;pins[name]=digest
 sourcepath=B/'baseline/covered.png';src=im(sourcepath);assert pins[str(sourcepath)]==project['source_sha256'];shape=src.shape[:2]
 def mask(path):return im(path)[:,:,0]>0
 lower=mask(W/'tree01-bark-seed-proposal-v1/proposed-visible-bark.png');upper=mask(W/'tree01-upper-bark-proposal-v1/proposed-upper-bark.png');old=lower|upper;accepted=mask(W/'tree01-role-exclusions-v1/all-phase-clear-bark-proposal.png');assert int(old.sum())==316 and int(accepted.sum())==294
 static=im(W/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png')[:,:,3]>0;staticfull=np.zeros(shape,bool);staticfull[:static.shape[0],:static.shape[1]]=static
 inventory=read(W/'tree01-canopy-source-v1/canopy-inputs.json');assert inventory['sprite']['position_x']==inventory['sprite']['position_y']==0;phases=[];native=np.zeros(shape,bool)
 for row in inventory['frames']:
  path=Path(row['path']);a=im(path);assert sha(path)==row['sha256'];full=np.zeros(shape,bool);full[:a.shape[0],:a.shape[1]]=a[:,:,3]>0;phases.append(full);native|=full
 assert len(phases)==14
 rawinventory=read(B/'source-inventory.json');assert sha(B/'source-inventory.json')==inventory['source_inventory_sha256'];rawanimation=next(a for a in rawinventory['animations']if a['index']==7);rawproof=[];rgba_phases=[]
 for i,row in enumerate(rawanimation['frames']):
  rawpath=Path(row['source']);raw=im(rawpath);assert sha(rawpath)==row['sha256'];normal=im(Path(row['image']));key=np.all(raw[:,:,:3]==[0,251,0],axis=2);expected=raw.copy();expected[key]=[0,0,0,0];assert np.array_equal(expected,normal);assert row['bbox']==[0,0,456,265];rgba_phases.append(normal);rawproof.append({'phase':i,'raw_path':str(rawpath),'raw_sha256':row['sha256'],'normalized_path':row['image'],'normalized_sha256':sha(Path(row['image'])),'full_rgba_conversion_exact':True,'key_pixels':int(key.sum()),'native_source_box':row['bbox'],'map_offset_xy':[0,0]})
 # Deliberately small manual interior patches beside reviewed spines. No color threshold or dilation.
 regions=[('central-left-exposed-interior',3,[(78,154),(79,154),(77,168),(76,172),(75,172),(76,165)]),('central-diagonal-face',4,[(112,176),(114,176),(117,185),(119,192),(117,192),(114,183)]),('central-lower-face',4,[(121,203),(122,203),(125,213),(127,220),(125,218),(123,211)]),('right-illuminated-face',5,[(147,164),(148,164),(147,173),(145,181),(144,181),(145,174)]),('lower-left-stem-interior',2,[(81,229),(83,231),(85,244),(84,248),(83,244),(81,234)])]
 union=np.zeros(shape,bool);rows=[]
 for name,context,polygon in regions:
  maskim=Image.new('L',(shape[1],shape[0]));ImageDraw.Draw(maskim).polygon(polygon,fill=255);raw=np.array(maskim)>0;candidate=raw&~old&~native&~staticfull;assert not np.any(candidate&(old|native|staticfull));union|=candidate
  rows.append({'id':name,'source_stem_context':context,'manual_polygon':polygon,'raw_pixels':int(raw.sum()),'old_spines_excluded':int((raw&old).sum()),'native_alpha_any_phase_excluded':int((raw&native).sum()),'static_proposal_excluded':int((raw&staticfull).sum()),'candidate_pixels_xy':[[int(x),int(y)]for y,x in np.argwhere(candidate)],'new_candidates':int(candidate.sum()),'per_phase_candidate_alpha_overlap':[int((candidate&a).sum())for a in phases]})
 oldconflict=accepted&native;failcoords=[]
 for y,x in np.argwhere(oldconflict):failcoords.append({'source_xy':[int(x),int(y)],'covered_phases':[i for i,a in enumerate(phases)if a[y,x]],'source_rgba':src[y,x].tolist(),'clear_phases':[i for i,a in enumerate(phases)if not a[y,x]],'all14_obscured':bool(all(a[y,x]for a in phases)),'native_rgba_by_phase':[a[y,x].tolist()for a in rgba_phases]})
 Image.fromarray(union.astype('uint8')*255).save(out/'proposed-additional-bark.png');rgba=np.zeros_like(src);rgba[union]=src[union];Image.fromarray(rgba).save(out/'proposed-source-rgba.png');assert np.array_equal(rgba[union],src[union]);marked=src[:,:,:3].copy();marked[accepted]=[50,200,255];marked[union]=[255,70,180];marked[oldconflict]=[255,255,255]
 crop=(60,120,165,260);sheet=Image.new('RGB',(1050,745),'#292929');sheet.paste(Image.fromarray(src).convert('RGB').crop(crop).resize((525,700),Image.Resampling.NEAREST),(0,25));sheet.paste(Image.fromarray(marked).crop(crop).resize((525,700),Image.Resampling.NEAREST),(525,25));d=ImageDraw.Draw(sheet);d.text((4,6),'Unmarked source',fill='white');d.text((530,6),'Pink new proposals; blue old294; white full-alpha conflicts',fill='white');d.text((5,730),'Manual stem interiors only. All pixels remain unassigned proposals.',fill='white');sheet.save(out/'source-proposal-comparison.png')
 phaseimage=Image.new('RGB',(7*210,2*300),'#303030')
 for i,a in enumerate(phases):
  context=src[:,:,:3].copy();context[a]=[40,170,230];context[union]=[255,70,180];context[accepted&a]=[255,255,255];pic=Image.fromarray(context).crop(crop).resize((210,280),Image.Resampling.NEAREST);phaseimage.paste(pic,((i%7)*210,(i//7)*300+20));ImageDraw.Draw(phaseimage).text(((i%7)*210+3,(i//7)*300+3),f'Full phase {i:02}',fill='white')
 phaseimage.save(out/'all14-phase-coverage.png');report={'status':'HUMAN_REVIEW_PROPOSALS_NO_ASSIGNMENTS','pins':pins,'full_animation_source':inventory,'raw_frame_conversion_proof':rawproof,'native_ownership_limit':'Full shared animation foreground coverage does not establish which physical tree owns those leaves; no reassignment of shared leaves to Tree01.','old_reviewed_samples':294,'old_excluded_samples_preserved':22,'old294_full_native_overlap_per_phase':[int((accepted&a).sum())for a in phases],'old294_full_native_overlap_union':int(oldconflict.sum()),'old294_full_native_overlap_details':failcoords,'new_candidate_count':int(union.sum()),'regions':rows,'observations':['Small manually outlined strips follow continuous longitudinal bark on the same reviewed stems; candidates stop before obvious yellow/green leaf clusters and bankstone.','The lower green foliage near left stem and its gaps remain unassigned; Tree00 overlap is not treated as bark even when shared native alpha is absent.','Dark stem flanks, cross-stem shadow and upper crown continuation remain unresolved. Alpha complement is never classified as background or bark.','Full shared Arbre08456x265 alpha is checked, not just the private Tree01 spatial fragment. Existing294 overlap outside that fragment is reported unchanged.'],'limits':['No saved receiver tests for new candidates; source review and exact saved firsthits required before any private projection.','Source-context numbers identify the traced stems, not final texture owner or new assignment.','All prior15 leaf-covered and7 nohit samples excluded through the full316old-spine mask, with zero candidate overlap.','Static6031 proposal is conservatively excluded without promoting it to authority.','No geometry, source ownership, existing masks, models or texture synthesis changed.']};(out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print({k:report[k]for k in ['new_candidate_count','old294_full_native_overlap_union','old294_full_native_overlap_per_phase']});print(failcoords)
if __name__=='__main__':main()

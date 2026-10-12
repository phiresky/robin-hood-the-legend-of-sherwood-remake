"""Freeze common-origin log-trap artwork and complete target action rows."""
from pathlib import Path
import hashlib,json,re
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
SRC=ROOT/'level-editor/work/croisement03-refinement/mission-endpoint-preparation-oct12-v1'
OUT=ROOT/'level-editor/work/croisement03-refinement/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=True);d=json.loads((SRC/'inventory.json').read_text());targets=[t for t in d['targets'] if 'buches01_b' in t['name']];crop=(420,380,720,620);pins={str(SRC/'inventory.json'):sha(SRC/'inventory.json')};records=[]
 def frame_image(t,f):
  p=SRC/f['display_path'];assert sha(p)==f['display_sha256'];pins[str(p)]=sha(p)
  inst=t['instances'][0]['source'];meta=t['profile_metadata'];origin=[inst['position_x']-meta['center_x'],inst['position_y']-meta['center_y']]
  assert origin==[469,424]
  for other in t['instances']:assert [other['source']['position_x']-meta['center_x'],other['source']['position_y']-meta['center_y']]==origin
  canvas=Image.new('RGBA',(300,240));canvas.alpha_composite(Image.open(p).convert('RGBA'),(int(origin[0]+f['offset_x']-crop[0]),int(origin[1]+f['offset_y']-crop[1])))
  return canvas
 for t in targets:
  row=next(r for r in t['rows'] if r['action_id']==160);name=t['name'].split(' - ')[1];n=len(row['frames']);sheet=Image.new('RGB',(1200,268*((n+3)//4)),'#303030');draw=ImageDraw.Draw(sheet);delay=0
  for i,f in enumerate(row['frames']):
   canvas=frame_image(t,f);x=i%4*300;y=i//4*268;sheet.paste(canvas,(x,y),canvas);draw.text((x+5,y+242),f'{name} frame {i}; row-delay sum {delay}; delay {f["delay"]}',fill='white');delay+=f['delay']
  sheet.save(OUT/f'{name}-full-action160.png');records.append({'profile':t['name'],'instances':t['instances'],'rows':t['rows'],'action160_total_authored_delay':delay,'timing_scope':'Authored delays; not a claimed runtime tick-to-frame mapping.'})
 mapfile=ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Day/Croisement03.map.png';pins[str(mapfile)]=sha(mapfile);background=Image.open(mapfile).convert('RGBA').crop(crop)
 for state,action in [('initial',0),('terminal',160)]:
  composite=Image.new('RGBA',(300,240))
  for t in targets:
   row=next(r for r in t['rows'] if r['action_id']==action);composite.alpha_composite(frame_image(t,row['frames'][0 if action==0 else -1]))
  composite.save(OUT/f'{state}-aligned.png');composite.resize((900,720),Image.Resampling.NEAREST).save(OUT/f'{state}-aligned-large.png');context=Image.alpha_composite(background,composite);context.resize((900,720),Image.Resampling.NEAREST).save(OUT/f'{state}-context.png')
  grid=composite.resize((1200,960),Image.Resampling.NEAREST);draw=ImageDraw.Draw(grid)
  for x in range(0,300,10):draw.line((x*4,0,x*4,960),fill=(255,255,255,50));draw.text((x*4+1,0),str(x+420),fill='red')
  for y in range(0,240,10):draw.line((0,y*4,1200,y*4),fill=(255,255,255,50));draw.text((0,y*4+1),str(y+380),fill='red')
  grid.save(OUT/f'{state}-survey-grid.png')
 # Emulate the source target freeze counter: reset sentinel wraps on the first update.
 timelines={}
 for target in targets:
  row=next(r for r in target['rows'] if r['action_id']==160);index=0;counter=65535;timeline=[]
  for tick in range(100):
   if tick and index<len(row['frames'])-1:
    counter=(counter+1)&65535
    if counter>row['frames'][index]['delay']:counter=0;index+=1
   timeline.append({'updates_since_reset':tick,'frame':index,'visible':bool(row['frames'][index]['effective_alpha_bbox'])})
  timelines[target['name']]=timeline
 scripts=[]
 for record in d['native_scripts']:
  if not record['path'].endswith('.ts'):continue
  file=SRC/record['path'];text=file.read_text();start=re.search(r'class buches_b1[^\n]+',text)
  if not start:continue
  block=text[start.start():];stop=block.find('\nclass ',10);block=block[:stop] if stop>=0 else block
  pins[str(file)]=sha(file);scriptfile=OUT/(record['mission']+'-logtrap-class.txt');scriptfile.write_text(block)
  actors=re.findall(r'RecordPlayAnimFreeze\(/\*actor\*/ GetActorScript\(/\*iPosition\*/ (\d+)\)',block)
  timers=re.findall(r'RecordTimer\(/\*iFrames\*/ (\d+)\)',block)
  scripts.append({'mission':record['mission'],'script_sha256':sha(file),'excerpt':scriptfile.name,'animation_actor_indices':list(map(int,actors)),'timers':list(map(int,timers)),'has_initial_deactivation':'Deactivate(' in block,'has_activation':'Activate(' in block})
 assert all(len(x['animation_actor_indices'])==3 for x in scripts)
 for tick in range(100):
  assert sum(row[tick]['visible'] for row in timelines.values())==1,(tick,'component lifetime overlap or gap')
 (OUT/'timing.json').write_text(json.dumps({'status':'source counter and script qualified; no physical motion rig','timelines':timelines,'scripts':scripts,'checks':{'one_visible_component_all_100_updates':True,'b1_to_b2_update':25,'b2_to_b3_update':40,'b3_terminal_update':88},'semantics':['Counter starts at65535; increment wraps to0; advance when counter strictly exceeds delay; final frame freezes.','Three PlayAnimFreeze commands terminate immediately after animation reset. Their following message2999 therefore does not wait for animation completion.','Timings are local sprite update counts after reset, not an asserted mission clock activation offset.','Timer60 and damage timers10/15/15 remain separate script sequences; they are not solid animation keyframes.']},indent=2)+'\n')
 (OUT/'source-qualification.json').write_text(json.dumps({'status':'source qualified, geometry unapproved','common_art_origin':[469,424],'crop':crop,'targets':records,'pins':pins,'receiver_scope':'buches_ombre is separate ground/shadow artwork; excluded from solid model.','lifetimes':{'initial':'b1 only; b2/b3 empty','terminal':'b3 only; b1/b2 empty','intermediate':'All full action160 rows retained; transient b2 is not an extra terminal solid.'},'limitations':['Authored row delay sums alone do not establish sequence start/end runtime offsets.','No physical animation correspondence inferred from endpoint log counts.']},indent=2)+'\n')
if __name__=='__main__':main()

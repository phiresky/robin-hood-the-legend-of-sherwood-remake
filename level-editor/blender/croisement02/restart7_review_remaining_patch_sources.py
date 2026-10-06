"""Render source-native phase review crops bound to the independent/runtime agreement."""
from pathlib import Path
import json,hashlib
from PIL import Image,ImageDraw,ImageOps
recipe=Path(__file__).with_name('restart7_reference_remaining_patches.py')
exec(compile(recipe.read_text().split('records=[]')[0],str(recipe),'exec'))
out=b/'source-review-v1';out.mkdir(exist_ok=True)
reference={r['id']:r for r in json.loads((b/'independent-reference.json').read_text())['records']}
comparison=json.loads((b/'parity-comparison.json').read_text());assert comparison['status']=='PASS'
chosen={}
for r in manifest['records']:chosen.setdefault(r['profile'],r)
def compose(n,selected,tick,terminal):
 dst=image(n['background']).copy();elements=[]
 if selected['integrate_in_background'] and tick>=terminal:paint(dst,selected['transition'][-1],selected['display_position'],n['origin'])
 for e in n['elements']:
  f=e.get('initial_frame') if not e['active'] else frame(e['frames'],max(0,tick),e['loop'])
  if e['active'] or f:elements.append({**e,'frame':f})
 for p in n['patch_states']:
  f=phase_frame(p,tick) if p['id']==selected['id'] else frame(p['initial'],max(0,tick),p['initial_loop'])
  if f is None:continue
  if p['layer']=='background':paint(dst,f,p['display_position'],n['origin'])
  else:elements.append({**p,'frame':f})
 for e in order(elements):paint(dst,e['frame'],e['display_position'],n['origin'])
 return dst
sheet=Image.new('RGB',(1120,len(chosen)*245),(38,38,38));draw=ImageDraw.Draw(sheet);records=[]
for row,(profile,r) in enumerate(chosen.items()):
 c=json.loads((b/r['contract']).read_text());n=c['native'];p=next(p for p in n['patch_states']if p['id']==c['focus_patch_id']);frames=p['initial']+p['transition']+p['final'];x=min(p['display_position'][0]+f['offset'][0] for f in frames);y=min(p['display_position'][1]+f['offset'][1] for f in frames);right=max(p['display_position'][0]+f['offset'][0]+f['width']for f in frames);bottom=max(p['display_position'][1]+f['offset'][1]+f['height']for f in frames);box=(max(0,x-12),max(0,y-12),min(1792,right+12),min(1152,bottom+12));draw.text((6,row*245+3),profile+' — source camera/art',fill='white')
 for column,(label,tick) in enumerate([('Initial',-1),('Transition',r['terminal_tick']//2),('Applied',r['terminal_tick']+2),('Reset',-1)]):
  pixels=compose(n,p,tick,r['terminal_tick']);h=hashlib.sha256(pixels.tobytes()).hexdigest();assert h==next(f['rgba_sha256']for f in reference[r['id']]['cases']if f['tick']==tick);crop=Image.fromarray(pixels).crop(box);name=f'{row}-{label.lower()}.png';crop.save(out/name);tile=ImageOps.contain(crop.convert('RGB'),(272,210),Image.Resampling.NEAREST);sheet.paste(tile,(column*280+(280-tile.width)//2,row*245+32));draw.text((column*280+5,row*245+19),label,fill='white');records.append({'id':r['id'],'profile':profile,'label':label,'tick':tick,'crop':list(box),'full_rgba_sha256':h,'image':name,'sha256':hashlib.sha256((out/name).read_bytes()).hexdigest()})
sheet.save(out/'sheet.png');(out/'manifest.json').write_text(json.dumps({'status':'SOURCE_REVIEW_PENDING','scope':'Eight distinct source profiles with initial/transition/applied/reset views. Full RGBA hashes match independent and production compositor cases. No fabricated physical endpoints or actor/gameplay claim.','profiles':len(chosen),'images':records,'parity_sha256':hashlib.sha256((b/'parity-comparison.json').read_bytes()).hexdigest()},indent=2)+'\n');print(out)

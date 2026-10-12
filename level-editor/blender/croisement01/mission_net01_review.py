"""Assemble immutable saved-model sheets and aligned source comparisons for net01."""
import argparse,hashlib,json,math,pathlib
from PIL import Image,ImageDraw
ROOT=pathlib.Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement01-refinement/mission-net01-oct12-v1'
def sha(p):return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--candidate',default='candidate-v1');args=ap.parse_args();p=OUT/args.candidate;r=json.loads((p/'report.json').read_text());s=json.loads((OUT/'source.json').read_text());assert sha(p/'model.blend')==r['model_sha256'];dest=p/'review';dest.mkdir(exist_ok=False);base=Image.open(OUT.parent/'baseline/covered.png').convert('RGBA');records=[]
 for state,keys in [('initial',['initial']),('empty',['g','e']),('occupied',['g','i'])]:
  for mode in ['actual','solid']:
   sheet=Image.new('RGBA',(1536,1064),(45,45,45,255));draw=ImageDraw.Draw(sheet)
   for view in range(8):
    f=next(x for x in r['renders'] if x['state']==state and x['mode']==mode and x['view']==view);assert sha(p/f['path'])==f['sha256'];im=Image.open(p/f['path']).convert('RGBA');x=(view%4)*384;y=(view//4)*532;sheet.alpha_composite(im,(x,y+20));draw.text((x+6,y+4),('ORIGINAL ART CAMERA' if view==0 else f'View {view}')+' '+state,fill='white')
   name=f'{state}-{mode}-eight.png';sheet.convert('RGB').save(dest/name);records.append({'path':name,'sha256':sha(dest/name)})
  src=Image.new('RGBA',base.size);context=base.copy()
  for key in keys:
   f=s['sources'][key];assert sha(f['path'])==f['sha256'];im=Image.open(f['path']).convert('RGBA');xy=tuple(int(f['position'][i]+f['offset'][i]) for i in [0,1]);src.alpha_composite(im,xy);context.alpha_composite(im,xy)
  cam=next(x for x in r['cameras'] if x['state']==state and x['view']==0);loc=[cam['matrix_world'][i][3] for i in range(3)];cy=-loc[1]*math.sin(math.radians(35))-loc[2]*math.cos(math.radians(35));h=cam['ortho_scale'];w=h*.75;box=(loc[0]-w/2,cy-h/2,loc[0]+w/2,cy+h/2);expected=src.transform((384,512),Image.Transform.EXTENT,box,resample=Image.Resampling.NEAREST);bg=context.transform((384,512),Image.Transform.EXTENT,box,resample=Image.Resampling.NEAREST);comparison=Image.new('RGBA',(1152,540),(45,45,45,255));draw=ImageDraw.Draw(comparison)
  for i,(im,label) in enumerate([(bg,'Native full context'),(expected,'Native net artwork only'),(Image.open(p/f'{state}-00-actual.png').convert('RGBA'),'Saved model original camera')]):comparison.alpha_composite(im,(i*384,24));draw.text((i*384+5,4),label,fill='white')
  name=f'{state}-native-comparison.png';comparison.convert('RGB').save(dest/name);records.append({'path':name,'sha256':sha(dest/name),'source_view_box':box,'resampling':'Nearest-neighbor diagnostic framing; raw source images remain unchanged.'})
 html=['<!doctype html><meta charset="utf-8"><title>C1 net01 private geometry</title><style>body{background:#222;color:#ddd;font:16px sans-serif;margin:24px}img{max-width:100%;display:block;margin-bottom:18px}section{border-top:2px solid #888}a{color:#cde}</style><h1>Crossings01 net01 — private geometry candidate</h1><p>Self-review in progress. Not ready for user approval. Initial, empty and occupied phase0 shapes are separate; final wobble motion and neighboring attachment validation remain separate. Unknown backs are gray. Ground sheets are labelled receiver domains, not new obstacles.</p>']
 if (p/'support-bindings.json').exists():
  html.append('<p>The upper-right rope and initial wood piece use source-aligned branch contacts. The short upper-left connector and rope depth are inferred and require review. Initial rope meets the ground receiver. Tree04 is a separate private candidate with its own pending approval; it is shown only as context. Animation, terrain partitioning and canonical integration are excluded.</p><p><a href="../support-bindings.json">Measured endpoint contacts and disclosed inference</a></p>')
  records.append({'path':'../support-bindings.json','sha256':sha(p/'support-bindings.json'),'role':'Endpoint-only attachment and clearance diagnostics'})
 for state in ['initial','empty','occupied']:
  html.append(f'<section><h2>{state}</h2>')
  for suffix in ['native-comparison','solid-eight','actual-eight']:html.append(f'<img src="{state}-{suffix}.png">')
  if (p/'tree04-contact.json').exists():
   contact=json.loads((p/'tree04-contact.json').read_text());context_sheet=Image.new('RGBA',(768,542),(45,45,45,255));draw=ImageDraw.Draw(context_sheet)
   for i,view in enumerate(['original','reverse']):
    name=f'{state}-tree04-context-{view}.png';record=next(x for x in contact['renders'] if x['path']==name);assert sha(p/name)==record['sha256'];context_sheet.alpha_composite(Image.open(p/name).convert('RGBA'),(i*384,30));draw.text((i*384+5,5),view+' / brown = pending Tree04 context',fill='white')
   name=f'{state}-tree04-diagnostic.png';context_sheet.convert('RGB').save(dest/name);records.append({'path':name,'sha256':sha(dest/name),'role':'Diagnostic exact evaluated private tree geometry; uniform brown is not approved appearance.'});html.append(f'<p>Exact evaluated Tree04 wood shown only as brown diagnostic context. Its geometry approval and canonical ownership transfer remain separate.</p><img src="{name}">')
  if (p/'counterweight-closeups.json').exists():
   closeups=json.loads((p/'counterweight-closeups.json').read_text())
   for mode in ['actual','solid']:
    wood=Image.new('RGBA',(1536,542),(45,45,45,255));draw=ImageDraw.Draw(wood)
    for view in range(4):
     f=next(x for x in closeups['renders'] if x['state']==state and x['mode']==mode and x['view']==view);assert sha(p/f['path'])==f['sha256'];wood.alpha_composite(Image.open(p/f['path']).convert('RGBA'),(view*384,30));draw.text((view*384+5,5),('Original direction' if view==0 else f'Quarter turn {view}')+' wood '+mode,fill='white')
    name=f'{state}-wood-{mode}-four.png';wood.convert('RGB').save(dest/name);records.append({'path':name,'sha256':sha(dest/name),'role':'Counterweight-only diagnostic closeups'});html.append(f'<img src="{name}">')
  if (p/'wood-oblique-diagnostics.json').exists():
   diagnostic=json.loads((p/'wood-oblique-diagnostics.json').read_text());assert diagnostic['model_sha256']==r['model_sha256']
   sheet=Image.new('RGBA',(1920,562),(45,45,45,255));draw=ImageDraw.Draw(sheet)
   for i,(mode,label) in enumerate([('actual','Actual: original cutoff 0.05'),('solid','Solid: all geometry'),('roles','Cyan core / orange source shell'),('core-only-solid','Core only: shell hidden'),('cutoff-0.3','Diagnostic cutoff 0.3 (not worker)')]):
    f=next(x for x in diagnostic['renders'] if x['state']==state and x['mode']==mode);assert sha(p/f['path'])==f['sha256'];sheet.alpha_composite(Image.open(p/f['path']).convert('RGBA'),(i*384,50));draw.text((i*384+5,5),label,fill='white')
   name=f'{state}-wood-oblique-diagnostics.png';sheet.convert('RGB').save(dest/name);records.append({'path':name,'sha256':sha(dest/name),'role':diagnostic['scope']});html.append(f'<p>{diagnostic["scope"]}</p><img src="{name}">')
  html.append('</section>')
 (dest/'index.html').write_text('\n'.join(html));(dest/'manifest.json').write_text(json.dumps({'model_sha256':r['model_sha256'],'report_sha256':sha(p/'report.json'),'source_sha256':sha(OUT/'source.json'),'resources':records,'status':'Self-review pending; no approval inferred'},indent=2)+'\n')
if __name__=='__main__':main()

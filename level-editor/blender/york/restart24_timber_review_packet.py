"""Build native and physical-contact evidence from a saved timber candidate."""
import hashlib,json,math,sys
from pathlib import Path
from PIL import Image,ImageDraw
from shapely.geometry import Polygon
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/sys.argv[1]
if(BASE/'packet-review.json').exists():raise FileExistsError(BASE/'packet-review.json')
r=json.loads((BASE/'candidate.json').read_text());pieces=r['pieces'];vv=[[x,y,p['bottom_z']]for p in pieces for x,y in p.get('bottom_footprint_world',p['footprint_world'])]+[[x,y,p['top_z']]for p in pieces for x,y in p['footprint_world']];s=math.sin(math.radians(35));c=math.cos(math.radians(35));center=[(min(v[k]for v in vv)+max(v[k]for v in vv))/2 for k in range(3)];cx=center[0];cy=-s*center[1]-c*center[2]
art=Image.open(WORK/'pair-v14/assets/york-southwest-square-west-house/reference/source.png').convert('RGB');native=art.transform((256,256),Image.Transform.EXTENT,(cx-50,cy-50,cx+50,cy+50),resample=Image.Resampling.NEAREST);actual=Image.open(BASE/'review/0-textured.png').convert('RGBA');im=Image.new('RGB',(512,280),(35,40,45));im.paste(native,(0,24));im.paste(actual,(256,24),actual);d=ImageDraw.Draw(im);d.text((5,5),'Native source / exact camera extent',fill='white');d.text((261,5),'Saved model / observed source faces',fill='white');im.save(BASE/'native-comparison.png')
# Matching native view and enlarged tip region, without inventing image content.
region=(40,110,175,190);detail=Image.new('RGB',(810,264),(35,40,45));detail.paste(native.crop(region).resize((405,240),Image.Resampling.NEAREST),(0,24));candidate=Image.new('RGB',(256,256),(35,40,45));candidate.paste(actual,mask=actual);detail.paste(candidate.crop(region).resize((405,240),Image.Resampling.NEAREST),(405,24));d=ImageDraw.Draw(detail);d.text((5,5),'Source crossing ends',fill='white');d.text((410,5),'Saved crossing ends',fill='white');detail.save(BASE/'crossing-end-detail.png')
contacts=[]
for a in pieces:
 for b in pieces:
  if a['id']==b['id']or abs(a['top_z']-b['bottom_z'])>1e-6:continue
  q=Polygon(a['footprint_world']).intersection(Polygon(b.get('bottom_footprint_world',b['footprint_world'])))
  if q.area>1e-8:contacts.append({'lower':a['id'],'upper':b['id'],'area':q.area,'polygon':list(q.exterior.coords)})
assert len(contacts)==7
xmin=min(v[0]for v in vv)-5;xmax=max(v[0]for v in vv)+5;ymin=min(v[1]for v in vv)-5;ymax=max(v[1]for v in vv)+5
im=Image.new('RGB',(660,510),(35,40,45));d=ImageDraw.Draw(im)
def screen(v):return((v[0]-xmin)/(xmax-xmin)*600+30,470-(v[1]-ymin)/(ymax-ymin)*400)
for q,color in zip(pieces,['#ff5050','#ffff00','#30ffff','#50ff50','#f080ff','#ffffff']):
 pts=[screen(v)for v in q.get('bottom_footprint_world',q['footprint_world'])];d.line(pts+[pts[0]],fill=color,width=2);d.text(pts[0],q['id'],fill=color)
for q in contacts:d.polygon([screen(v)for v in q['polygon']],fill='#405040',outline='#80ff80')
for q in r['ground_queries']:
 x,y=screen(q['foot']);d.ellipse((x-2,y-2,x+2,y+2),fill='orange')
d.text((15,10),'Orange: fresh receiver queries. Green: actual bottom/top contact footprints.',fill='white');d.text((15,28),'Contact areas do not certify mechanical stability.',fill='white');im.save(BASE/'contact-diagnostic.png')
report={'status':'PRIVATE_PACKET_REQUIRES_SOURCE_BOUNDARY_REVIEW','model_sha256':r['model_sha256'],'source':r['source_diagnostic'],'ground_samples':len(r['ground_queries']),'max_abs_ground_delta':max(abs(q['delta_z'])for q in r['ground_queries']),'contacts':contacts,'images':{name:hashlib.sha256((BASE/name).read_bytes()).hexdigest()for name in['solid8.png','textured8.png','native-comparison.png','crossing-end-detail.png','contact-diagnostic.png']},'bytes':sum(p.stat().st_size for p in BASE.rglob('*')if p.is_file())};(BASE/'packet-review.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'base':str(BASE),'bytes':report['bytes'],'ground_delta':report['max_abs_ground_delta']}))

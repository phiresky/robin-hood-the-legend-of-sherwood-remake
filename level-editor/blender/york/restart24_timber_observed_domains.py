"""Trace complete visible wood faces; preserve rejected and ambiguous regions."""
import json,hashlib
from pathlib import Path
from PIL import Image,ImageDraw
from shapely.geometry import Polygon,Point
from shapely.ops import unary_union
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-observed-domains-v1'
if OUT.exists():raise FileExistsError(OUT)
prior=json.loads((WORK/'loose-planks-ownership-v3/report.json').read_text());rows=[]
tops={r['piece']:Polygon(r['polygon'])for r in prior['domains']if r['role']=='top'}
long_outline=[(1264,928),(1268,929),(1294,955),(1294,961),(1292,965),(1290,961),(1288,959),(1286,956),(1284,954),(1282,951),(1280,949),(1279,948),(1278,947),(1277,946),(1276,945),(1276,944),(1275,943),(1274,942),(1273,941),(1273,940),(1272,939),(1272,938),(1271,937),(1270,936),(1269,935),(1268,934),(1267,933),(1266,932),(1265,931),(1264,930)]
long_shape=Polygon(long_outline);short_shape=Polygon([(1250,944),(1277,955),(1277,958),(1276,960),(1274,959),(1250,949)])
assert long_shape.is_valid and short_shape.is_valid
crossing=unary_union([long_shape,short_shape])
side_bands={
 'pale-1':[(1257,952),(1285,934),(1285,936),(1257,954)],
 'pale-2':[(1265,956),(1293,938),(1293,940),(1265,958)],
 'pale-3':[(1272,960),(1300,942),(1300,944),(1272,962)],
 'pale-4':[(1287,964),(1310,947),(1310,949),(1287,966)]}
for piece,top in tops.items():
 if piece.startswith('pale'):
  end=next(r for r in prior['domains']if r['piece']==piece and r['role']=='side')
  side=Polygon(side_bands[piece])
  if piece!='pale-4':side=side.union(Polygon(end['polygon']))
  visible_top=top.difference(crossing);visible_side=side.difference(top).difference(crossing)
 else:
  silhouette=long_shape if piece=='long-crossing'else short_shape
  visible_top=top.intersection(silhouette);visible_side=silhouette.difference(top)
 rows.extend([{'piece':piece,'role':'top','shape':visible_top},{'piece':piece,'role':'side','shape':visible_side}])
OUT.mkdir();(OUT/'domains').mkdir();owners={};conflicts=[]
for y in range(920,978):
 for x in range(1240,1320):
  point=Point(x+.5,y+.5);claims=[(r['piece'],r['role'])for r in rows if r['shape'].covers(point)]
  if len(claims)==1:owners[(x,y)]=claims[0]
  elif len(claims)>1:conflicts.append({'pixel':[x,y],'claims':claims})
for r in rows:
 im=Image.new('L',(80,58));key=(r['piece'],r['role'])
 for(x,y),o in owners.items():
  if o==key:im.putpixel((x-1240,y-920),255)
 im.save(OUT/'domains'/f'{r["piece"]}-{r["role"]}.png');r['pixels']=sum(o==key for o in owners.values());r['polygon_wkt']=r.pop('shape').wkt
source=WORK/'pair-v14/assets/york-southwest-square-west-house/reference/source.png';art=Image.open(source).convert('RGB').crop((1240,920,1320,978));known=Image.new('RGB',(80,58),(100,100,100));rolemap=Image.new('RGB',(80,58),(35,40,45))
for(x,y),o in owners.items():
 known.putpixel((x-1240,y-920),art.getpixel((x-1240,y-920)));rolemap.putpixel((x-1240,y-920),(70,220,100)if o[1]=='top'else(240,160,40))
known.resize((800,580),Image.Resampling.NEAREST).save(OUT/'observed-wood.png');rolemap.resize((800,580),Image.Resampling.NEAREST).save(OUT/'top-side-roles.png')
report={'status':'PRIVATE_FULL_VISIBLE_FACE_TRACE_REVIEW_REQUIRED','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'domains':rows,'pixels':[{'pixel':xy,'owner':o}for xy,o in owners.items()],'unresolved_overlap_pixels':conflicts,'long_visible_outline':long_outline,'known_pixel_count':len(owners),'rules':['No eroded-core substitute; all traced visible face pixels are targets.','Stepped long-side outline traced against enlarged native grid; pale-yellow gaps remain with their boards.','Black/brown edge strips are distinguished from the broad blue ground beyond them.','Ambiguous shared-domain pixels remain unresolved, not silently assigned.'],'geometry_hypothesis':{'pale_board_thickness':4,'long_crossing_thickness':3,'short_crossing_thickness':4,'basis':'Visible side/end strips span roughly2..3sourcepixels. These hypotheses require source and contact checks.'}}
(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'out':str(OUT),'pixels':len(owners),'conflicts':len(conflicts)}))

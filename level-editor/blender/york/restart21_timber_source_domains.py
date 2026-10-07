"""Independent per-piece wood domains and diagnostic candidate coverage.

Manual visible-face hypotheses remain private. Uncertain edges do not become
permission to fill source artwork. No mesh, material or library mutation.
"""
import hashlib,json
from pathlib import Path
from PIL import Image,ImageDraw
from shapely.geometry import Polygon,Point
from shapely.ops import unary_union
ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement/restart2'
OUT=WORK/'loose-planks-ownership-v3'
MODEL=WORK/'loose-planks-candidate-v1'
SOURCE=WORK/'pair-v14/assets/york-southwest-square-west-house/reference/source.png'
if OUT.exists():raise FileExistsError(OUT)
assert hashlib.sha256((MODEL/'model.blend').read_bytes()).hexdigest()=='5d12fd9a2df711a302547b425c6e1a2fbe5d64389a247047f3e636c8bd98f70b'
assert hashlib.sha256(SOURCE.read_bytes()).hexdigest()=='3d83bfab6b8ff1c27f87c82c0c034dc854df44b4e2322a09e7e083db57fb409e'
box=(1240,920,1320,978);size=(80,58)
# Top boundaries are traced in the art independently of the saved mesh polygons.
specs=[
 ('pale-1','top',[(1253,949),(1257,952),(1285,934),(1281,931)]),
 ('pale-1','side',[(1253,951),(1257,953),(1257,955),(1253,953)]),
 ('pale-2','top',[(1261,953),(1265,956),(1293,938),(1289,935)]),
 ('pale-2','side',[(1261,954),(1265,957),(1265,960),(1261,957)]),
 ('pale-3','top',[(1268,957),(1272,960),(1300,942),(1296,939)]),
 ('pale-3','side',[(1268,958),(1272,960),(1272,963),(1268,961)]),
 ('pale-4','top',[(1282,960),(1287,964),(1310,947),(1306,944)]),
 ('pale-4','side',[(1283,964),(1287,966),(1287,968),(1283,966)]),
 ('long-crossing','top',[(1264,928),(1268,929),(1294,955),(1291,958)]),
 ('long-crossing','side',[(1291,958),(1294,955),(1294,961),(1292,965)]),
 ('short-crossing','top',[(1250,944),(1277,955),(1276,957),(1250,946)]),
 ('short-crossing','side',[(1250,946),(1276,957),(1276,960),(1250,949)]),
]
rows=[{'piece':p,'role':r,'polygon':v,'shape':Polygon(v)}for p,r,v in specs]
assert all(r['shape'].is_valid for r in rows)
crossing=unary_union([r['shape']for r in rows if 'crossing'in r['piece']])
blue=Polygon([(1297,958),(1306,953),(1314,952),(1314,958),(1305,963),(1296,967)])
for row in rows:
    visible=row['shape']
    if row['piece']=='pale-4' and row['role']=='side':
        visible=Polygon()
        row['excluded_reason']='Candidate end-face trace falls on ambiguous green/blue boundary, not confidently observed wood; retain no known pixels.'
    if row['piece'].startswith('pale'):visible=visible.difference(crossing)
    row['visible']=visible.difference(blue)
    row['core']=row['visible'].buffer(-.65)
OUT.mkdir();(OUT/'domains').mkdir()
art=Image.open(SOURCE).convert('RGBA').crop(box)
owner_at={};ambiguous=[];expected={};union_core=Image.new('L',size)
for y in range(size[1]):
 for x in range(size[0]):
    point=Point(x+box[0]+.5,y+box[1]+.5)
    claims=[(r['piece'],r['role'])for r in rows if r['core'].covers(point)]
    if len(claims)==1:owner_at[(x,y)]=claims[0];union_core.putpixel((x,y),255)
    elif len(claims)>1:ambiguous.append({'pixel':[x+box[0],y+box[1]],'claims':claims})
for row in rows:
    key=(row['piece'],row['role']);im=Image.new('L',size)
    for xy,owner in owner_at.items():
        if owner==key:im.putpixel(xy,255)
    path=OUT/'domains'/f'{row["piece"]}-{row["role"]}.png';im.save(path)
    expected[key]={xy for xy,owner in owner_at.items()if owner==key}
    row['conservative_core_pixels']=len(expected[key])
    row['mask_sha256']=hashlib.sha256(path.read_bytes()).hexdigest()
    for k in ('shape','visible','core'):del row[k]
union_core.save(OUT/'diagnostic-core-union.png')
# Original per-face source diagnostic encoded accepted source pixels over gray.
# Face1 is top in the pinned prism recipe; faces2..5 are sides,0 is bottom.
accepted={};incorrect=[];kept=set()
for path in sorted((MODEL/'face-source').glob('*.png')):
    piece,face=path.stem.rsplit('-',1);face=int(face);role='top'if face==1 else 'side'if face>1 else 'bottom'
    im=Image.open(path).convert('RGBA');count=0;keep=0
    for y in range(size[1]):
     for x in range(size[0]):
        pixel=im.getpixel((x,y))
        if pixel==(128,128,128,255):continue
        count+=1
        if owner_at.get((x,y))==(piece,role):keep+=1;kept.add((x,y))
        else:incorrect.append({'pixel':[x+box[0],y+box[1]],'candidate_piece':piece,'candidate_role':role,'independent_owner':owner_at.get((x,y))})
    accepted[path.name]={'old_accepted':count,'matches_independent_core':keep}
missing=[{'pixel':[x+box[0],y+box[1]],'owner':owner}for (x,y),owner in owner_at.items()if (x,y)not in kept]
colors={'pale-1':(255,80,80),'pale-2':(255,240,40),'pale-3':(40,230,250),'pale-4':(80,255,90),'long-crossing':(240,120,255),'short-crossing':(255,255,255)}
overlay=art.convert('RGB').resize((800,580),Image.Resampling.NEAREST);d=ImageDraw.Draw(overlay)
for row in rows:
    pts=[((x-box[0]+.5)*10,(y-box[1]+.5)*10)for x,y in row['polygon']]
    color=colors[row['piece']];d.line(pts+[pts[0]],fill=color,width=2 if row['role']=='top'else 1)
overlay.save(OUT/'independent-face-domains.png')
diff=Image.new('RGB',size,(35,40,45))
for (x,y),owner in owner_at.items():diff.putpixel((x,y),(60,220,90)if(x,y)in kept else(255,60,60))
for record in incorrect:
 x,y=record['pixel'];xy=(x-box[0],y-box[1])
 if xy not in owner_at:diff.putpixel(xy,(100,130,255))
diff.resize((800,580),Image.Resampling.NEAREST).save(OUT/'coverage-difference.png')
report={'status':'PRIVATE_MANUAL_DOMAIN_HYPOTHESIS_REQUIRES_VISUAL_CORRECTION',
 'model_sha256':hashlib.sha256((MODEL/'model.blend').read_bytes()).hexdigest(),
 'source_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'crop':box,
 'domain_semantics':'Per-piece per-face native artwork traces, independent of mesh acceptance; explicit crossing occlusion and blue-ground exclusion. Core erosion0.65px; uncertain/overlapping pixels are not texture-fill permission.',
 'blue_ground_exclusion':list(blue.exterior.coords),'domains':rows,'ambiguous_claims':ambiguous,
 'independent_core_pixels':len(owner_at),'candidate_matching_core':len(kept),
 'independent_wood_not_matched':missing,'candidate_pixels_not_matching_core':incorrect,'per_face':accepted,
 'geometry_preserved':True,'ground_queries_preserved':36,'contacts_preserved':7,
 'limits':['Manual domains still need visual refinement; counts are hypothesis diagnostics, not final coverage certification.',
 'Pixels outside conservative cores include legitimate wood boundary pixels as well as ground; rejected count is not a foreign-ground count.',
 'Unclassified dark pixels may be wood, shadow or ground; remain explicit unknown.',
 'Source gray128 pixels cannot be distinguished from unknown in old face PNGs; negligible but no exact completeness claim.',
 'Top-versus-side identity follows pinned prism face order; next Blender pass must verify normals.',
 'Short crossing timber tip thickness and end shape remain unresolved; do not extend a flat top across its end face.']}
(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'output':str(OUT),'core':len(owner_at),'kept':len(kept),'missing':len(missing),'rejected':len(incorrect),'ambiguous':len(ambiguous)}))

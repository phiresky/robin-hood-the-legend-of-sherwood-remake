"""Export reviewed room footprints, exact mechanical/door alpha masks and independent state canvases.

Run from the repository root before freezing source mask authority.
"""
from pathlib import Path
from PIL import Image,ImageDraw
import json,hashlib
root=Path('level-editor/work/needs-review/nottingham-refinement');out=root/'state-review/owned-masks';out.mkdir(parents=True,exist_ok=True);source=root/'source-states';layers=json.load(open(source/'layers.json'));base=Image.open(source/'revealed.png').convert('RGBA');covered=Image.open(source/'covered.png').convert('RGB')
polygons={
 'church-nave-floor':[(1764,1039),(1802,1061),(1856,1092),(1892,1071),(1969,1028),(2004,1048),(2109,987),(2155,1014),(2242,965),(2164,928),(2203,860),(2155,829),(2104,819),(2013,841),(1906,881),(1694,999)],
 'church-side-room-floor':[(1885,1088),(2039,1176),(2073,1169),(2133,1132),(2062,1091),(2044,1080),(1972,1039),(1935,1060),(1903,1077)],
 'mechanism-room-floor':[(1003,1417),(1048,1418),(1077,1429),(1077,1458),(1043,1475),(1011,1474),(977,1451),(977,1432)]}
records=[]
for name,points in polygons.items():
 mask=Image.new('L',base.size);ImageDraw.Draw(mask).polygon(points,fill=255);path=out/(name+'.png');mask.save(path);x0=min(p[0]for p in points)-30;y0=min(p[1]for p in points)-30;x1=max(p[0]for p in points)+30;y1=max(p[1]for p in points)+30
 preview=base.convert('RGB').crop((x0,y0,x1,y1));d=ImageDraw.Draw(preview);d.line([(x-x0,y-y0)for x,y in points]+[(points[0][0]-x0,points[0][1]-y0)],fill='cyan',width=2);preview.save(out/(name+'-review.png'))
 records.append({'id':name,'png':str(path.resolve()),'box':[0,0,*base.size],'polygon':points,'reviewed':True,'source_sha256':hashlib.sha256((source/'revealed.png').read_bytes()).hexdigest(),'review_evidence':str((out/(name+'-review.png')).resolve()),'scope':'Authored room footprint; actual visible ownership also requires retained-shell and furniture occlusion.'})
for patch,stem,node in [(3,'portcullis',333),(4,'mechanism',337)]:
 for state in ['initial','applied']:
  canvas=base.copy()
  for i,r in enumerate(layers['patches']):
   if patch==4 and i==5:continue
   graphic=r[state+'_graphic'] if i==patch else r['initial_graphic']
   if graphic:canvas.alpha_composite(Image.open(source/graphic['image']).convert('RGBA'),tuple(graphic['bbox'][:2]))
  name=stem+'-'+state;imagepath=out/(name+'-source.png');canvas.convert('RGB').save(imagepath)
  graphic=layers['patches'][patch][state+'_graphic'];alpha=Image.open(source/graphic['image']).convert('RGBA').getchannel('A');mask=Image.new('L',base.size);mask.paste(alpha,tuple(graphic['bbox'][:2]));maskpath=out/(name+'-mask.png');mask.save(maskpath)
  records.append({'id':name,'png':str(maskpath.resolve()),'box':[0,0,*base.size],'reviewed':True,'source_image':str(imagepath.resolve()),'source_sha256':hashlib.sha256(imagepath.read_bytes()).hexdigest(),'source_node':f'building-{node:03}','projection_component':name,'scope':'Exact state sprite opaque alpha, with source canvas recomposed from authoritative frames. Hidden depth not claimed.'})
(out/'manifest.json').write_text(json.dumps({'records':records},indent=2)+'\n')
# A deterministic inspection combination, not a mission reachability assertion.
canvas=base.copy()
for i,state in [(1,'applied'),(3,'initial'),(4,'initial'),(6,'applied')]:
 g=layers['patches'][i][state+'_graphic'];canvas.alpha_composite(Image.open(source/g['image']).convert('RGBA'),tuple(g['bbox'][:2]))
common=out/'revealed-with-open-doors-and-initial-mechanisms.png';canvas.convert('RGB').save(common)
common_record={'image':str(common.resolve()),'sha256':hashlib.sha256(common.read_bytes()).hexdigest(),'selection':{'room_covers':'all absent','patch-001':'applied','patch-003':'initial','patch-004':'initial','patch-006':'applied'},'limitation':'Independent diagnostic state selection; no mission reachability assertion.'}
for r in records:
 if 'floor' in r['id']:r['source_sha256']=common_record['sha256']
for patch,cover,stem,node in [(1,2,'upper-prison-door',453),(6,7,'southwest-prison-door',476)]:
 for state in ['initial','applied']:
  canvas=base.copy()
  for i,r in enumerate(layers['patches']):
   if i==cover:continue
   graphic=r[state+'_graphic'] if i==patch else r['initial_graphic']
   if graphic:canvas.alpha_composite(Image.open(source/graphic['image']).convert('RGBA'),tuple(graphic['bbox'][:2]))
  name=stem+'-'+state;imagepath=out/(name+'-source.png');canvas.convert('RGB').save(imagepath)
  graphic=layers['patches'][patch][state+'_graphic'];alpha=Image.open(source/graphic['image']).convert('RGBA').getchannel('A');mask=Image.new('L',base.size);mask.paste(alpha,tuple(graphic['bbox'][:2]));maskpath=out/(name+'-mask.png');mask.save(maskpath)
  records.append({'id':name,'png':str(maskpath.resolve()),'box':[0,0,*base.size],'reviewed':True,'source_image':str(imagepath.resolve()),'source_sha256':hashlib.sha256(imagepath.read_bytes()).hexdigest(),'source_node':f'building-{node if state=="initial" else (455 if patch==1 else 477):03}','scope':'Exact independent door endpoint sprite alpha; room cover absent, all unrelated initial patch graphics retained.'})
(out/'manifest.json').write_text(json.dumps({'records':records,'common_revealed_source':common_record},indent=2)+'\n')
print(json.dumps(common_record))

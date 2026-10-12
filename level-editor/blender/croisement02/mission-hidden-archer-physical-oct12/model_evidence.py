"""CPU source-plane geometry diagnostics; no depth, alpha or beauty-render claim."""
import json,struct,math
import numpy as np
from PIL import Image,ImageDraw
import qualify as q
RESOURCES={}

def primitives(candidate):
 data=(q.LIB/candidate['model']).read_bytes();n=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+n]);binary=data[28+n:];buffers=[(q.LIB/candidate['model']).parent.joinpath(b['uri']).read_bytes() if 'uri'in b else binary for b in doc['buffers']]
 for b in doc['buffers']:
  if 'uri'in b:
   path=(q.LIB/candidate['model']).parent.joinpath(b['uri']).resolve();RESOURCES[str(path)]=q.pin(path)
 def array(index):
  a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];assert 'sparse' not in a
  typ={5121:'<u1',5123:'<u2',5125:'<u4',5126:'<f4'}[a['componentType']];width={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];item=np.dtype(typ).itemsize
  return np.ndarray((a['count'],width),dtype=typ,buffer=buffers[v.get('buffer',0)],offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',width*item),item))
 def walk(i,parent):
  node=doc['nodes'][i];mat=parent@q.matrix(node)
  if 'mesh'in node:
   for p in doc['meshes'][node['mesh']]['primitives']:
    assert p.get('mode',4)==4;v=array(p['attributes']['POSITION']);v=(np.c_[v,np.ones(len(v))]@mat.T)[:,:3];v=v[:,[0,2,1]]*np.array([1,-1,1]);t=candidate['placement']['transform'];s,c=math.sin(math.radians(35)),math.cos(math.radians(35));v+=np.array([t['dx'],-t['dy']/s,t['dz']]);uv=np.c_[v[:,0],-v[:,1]*s-v[:,2]*c];idx=array(p['indices']).reshape(-1,3) if 'indices'in p else np.arange(len(v)).reshape(-1,3)
    yield node.get('name',str(i)),uv,idx
  for child in node.get('children',[]):yield from walk(child,mat)
 for i in doc['scenes'][doc.get('scene',0)]['nodes']:yield from walk(i,np.eye(4))

def main():
 report=json.loads((q.OUT/'qualification.json').read_text());background=Image.open(q.ROOT/report['background']['path']).convert('RGB');rows=[]
 for number,row in enumerate(report['rows'],1):
  x0,y0,x1,y1=row['source_union_box'];margin=35;crop=[x0-margin,y0-margin,x1+margin,y1+margin];w,h=crop[2]-crop[0],crop[3]-crop[1];cards=[];evidence=[]
  for candidate in row['bounds_candidates']:
   mask=Image.new('L',(w,h));draw=ImageDraw.Draw(mask);nodes=[];triangles=0
   for name,uv,idx in primitives(candidate):
    projected=uv[idx];keep=(projected[:,:,0].max(1)>=crop[0])&(projected[:,:,0].min(1)<crop[2])&(projected[:,:,1].max(1)>=crop[1])&(projected[:,:,1].min(1)<crop[3]);tri=projected[keep]-np.array(crop[:2]);triangles+=len(tri)
    if len(tri):nodes.append(name)
    for t in tri:draw.polygon([tuple(p) for p in t],fill=255)
   a=np.array(mask);count=int((a[margin:margin+y1-y0,margin:margin+x1-x0]>0).sum())
   if not count:continue
   card=background.crop(crop).convert('RGBA');overlay=Image.new('RGBA',(w,h),(0,180,255,0));overlay.putalpha(mask.point(lambda p:70 if p else 0));card=Image.alpha_composite(card,overlay).convert('RGB');d=ImageDraw.Draw(card);d.rectangle([margin,margin,x1-x0+margin,y1-y0+margin],outline='yellow');cards.append((candidate['id'],card));evidence.append(dict(asset_id=candidate['id'],model_sha256=candidate['model_sha256'],nodes=nodes,projected_triangles=triangles,conservative_covered_source_box_pixels=count))
  sheet=Image.new('RGB',(w*3,(h+28)*math.ceil(len(cards)/3)),(30,30,30));draw=ImageDraw.Draw(sheet)
  for i,(name,card) in enumerate(cards):x=(i%3)*w;y=(i//3)*(h+28);sheet.paste(card,(x,y+28));draw.text((x+3,y+4),name.replace('croisement02-',''),fill='white')
  path=q.OUT/f'archer{number:02}-current-geometry-projection.png';sheet.save(path);rows.append(dict(profile=row['profile'],evidence=q.pin(path),receivers=evidence))
 (q.OUT/'model-evidence.json').write_text(json.dumps(dict(qualification=q.pin(q.OUT/'qualification.json'),rows=rows,external_geometry_resources=list(RESOURCES.values()),method='All source-plane triangle footprints, including back and alpha-discarded faces; cyan shows conservative projected geometry, yellow the patch box. No first-hit, alpha visibility, depth contact or appearance claim.'),indent=2)+'\n')
 print([(r['profile'],[x['asset_id'] for x in r['receivers']])for r in rows])
if __name__=='__main__':main()

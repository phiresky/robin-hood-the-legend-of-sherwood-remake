"""Fit one rotating solid sign hypothesis to all native pose silhouettes."""
import json
import math
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from scipy.optimize import minimize
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
OUT=ROOT/'level-editor/work/croisement02-refinement'
SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))
FACES=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]

def box(x0,x1,y0,y1,z0,z1):
 return np.array([(x0,y0,z0),(x1,y0,z0),(x1,y1,z0),(x0,y1,z0),(x0,y0,z1),(x1,y0,z1),(x1,y1,z1),(x0,y1,z1)],float)

def geometry(p):
 width,height,bottom,postheight,postwidth,thick,by,angle,cx,cy=p[:10]
 by=-(postwidth+thick)/2+.15
 parts=[box(-width/2,width/2,by-thick/2,by+thick/2,bottom,bottom+height),box(-postwidth/2,postwidth/2,-postwidth/2,postwidth/2,0,postheight)]
 if len(p)>10:
  for part in parts:part[:,1]+=p[10]*part[:,2]/postheight
 return parts

def projected(vertices,p,frame):
 angle=math.radians(p[7]-frame*11.25);c,s=math.cos(angle),math.sin(angle)
 x=vertices[:,0]*c-vertices[:,1]*s;y=vertices[:,0]*s+vertices[:,1]*c
 return np.column_stack((x+p[8]+32,-SIN*y-COS*vertices[:,2]+p[9]+55))

def raster(p,frame,scale=2):
 im=Image.new('L',(64*scale,72*scale));dr=ImageDraw.Draw(im)
 for vertices in geometry(p):
  coords=projected(vertices,p,frame)*scale
  for face in FACES:dr.polygon([tuple(coords[i])for i in face],fill=255)
 return np.asarray(im.resize((64,72),Image.Resampling.BOX))>=128

def main():
 dst=OUT/'state-sign-candidate/fit-v6';dst.mkdir(exist_ok=False,parents=True)
 manifest=OUT/'state-target-evidence/manifest.json';data=json.loads(manifest.read_text());profile=next(p for p in data['profiles']if p['profile']=='Panneau');row=next(r for r in profile['rows']if r['action_id']==0)
 expected=[];sources=[]
 for f in row['frames']:
  path=Path(f['image']);assert sha(path)==f['image_sha256'];sprite=Image.open(path).convert('RGBA');im=Image.new('RGBA',(64,72));im.paste(sprite,(32+int(f['offset'][0]),55+int(f['offset'][1])));a=np.asarray(im);yy,xx=np.indices((72,64));body=(yy<43)|((np.abs(xx-32)<4)&(yy<60))|(a[:,:,:3].max(axis=2)>16);expected.append((a[:,:,3]>127)&body);sources.append(im)
 expected=np.stack(expected)
 def loss(p):
  rendered=np.stack([raster(p,i)for i in range(32)]);return 1-(rendered&expected).sum()/(rendered|expected).sum()
 initial=[40,21,21,48,4,2,-3,-34,0,1,5]
 bounds=[(39,45),(16,25),(15,23),(42,49),(3,6),(1,4),(-5,-1),(-40,-26),(-2,2),(-1,6),(-8,8)]
 result=minimize(loss,initial,method='Powell',bounds=bounds,options={'maxiter':10,'xtol':.03,'ftol':.00003});p=result.x;p[6]=-(p[4]+p[5])/2+.15;predicted=np.stack([raster(p,i)for i in range(32)])
 sheet=Image.new('RGB',(8*192,4*240),(70,70,70));metrics=[]
 for i,source in enumerate(sources):
  a=np.asarray(source.convert('RGB')).copy();a[expected[i]&~predicted[i]]=[255,35,35];a[predicted[i]&~expected[i]]=[35,100,255];tile=Image.fromarray(a).resize((192,216),Image.Resampling.NEAREST);sheet.paste(tile,((i%8)*192,(i//8)*240));ImageDraw.Draw(sheet).text(((i%8)*192+5,(i//8)*240+219),str(i),fill='white');metrics.append(dict(frame=i,expected=int(expected[i].sum()),missing=int((expected[i]&~predicted[i]).sum()),extra=int((predicted[i]&~expected[i]).sum()),iou=float((predicted[i]&expected[i]).sum()/(predicted[i]|expected[i]).sum())))
 sheet.save(dst/'source-silhouette-comparison.png')
 write_json(dst/'fit.json',dict(status='Private solid sign hypothesis; independent source/actual model review pending',parameters=dict(zip(['board_width','board_height','board_bottom','post_height','post_width','board_thickness','board_y','initial_angle_degrees','source_x_offset','source_y_offset','post_top_lean_y'],map(float,p))),aggregate_iou=1-loss(p),frames=metrics,manifest=str(manifest),manifest_sha256=sha(manifest),profile=profile,instances=[i for i in data['instances']if i['profile_id']==profile['id']],shadow='Black native shadow excluded from body fit and retained separately; no shadow-shaped geometry',timing='Each native pose lasts 2 ticks at 25Hz; 32 poses loop in 2.56 seconds while target active. Three action-specific artwork rows preserved.',limitations=['Hidden board thickness and post cross-section inferred from visible side poses.','Pixelated timber contour is not replaced by a mesh per sprite pixel.','Body/shadow separation is provisional: dark upper body retained; lower detached black shadow excluded by contextual pixel region.','All five native initial actions are0; other row appearances retained without invented triggers.']))
 print(dst,1-loss(p),dict(zip(['width','height','bottom','postheight','postwidth','thickness','boardy','angle','cx','cy','lean_y'],p)))

if __name__=='__main__':main()

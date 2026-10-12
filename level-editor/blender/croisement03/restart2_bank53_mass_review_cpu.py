"""Lightweight projected triangle diagrams; these are not saved-model render approval."""
import argparse,hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw

def main():
 p=argparse.ArgumentParser();p.add_argument('--geometry',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);data=json.loads(a.geometry.read_text());v=np.array(data['vertices']);f=np.array(data['faces']);normals=np.cross(v[f[:,1]]-v[f[:,0]],v[f[:,2]]-v[f[:,0]]);normals/=np.linalg.norm(normals,axis=1)[:,None];center=(v.min(0)+v.max(0))/2;scale=np.linalg.norm(v.max(0)-v.min(0));sheet=Image.new('RGB',(1200,640),'#303030');light=np.array([-.4,-.6,.7]);light/=np.linalg.norm(light)
 for i in range(8):
  theta=math.radians(i*45);s=math.sin(math.radians(35));c=math.cos(math.radians(35));direction=np.array([math.sin(theta)*c,-math.cos(theta)*c,s]);right=np.array([math.cos(theta),math.sin(theta),0]);up=np.cross(direction,right);xy=np.column_stack(((v-center)@right,-(v-center)@up))*260/scale+[150,160];depth=v@direction;tile=Image.new('RGB',(300,320),'#303030');draw=ImageDraw.Draw(tile)
  for j in np.argsort(depth[f].mean(1)):
   if normals[j]@direction<=0:continue
   value=int(70+150*max(0.,normals[j]@light));draw.polygon([tuple(q)for q in xy[f[j]]],fill=(value,value,value))
  draw.text((4,4),f'CPU triangle diagram {i*45}deg',fill='white');sheet.paste(tile,((i%4)*300,(i//4)*320))
 sheet.save(a.output/'cpu-eight.png')
 front=f[np.all(f<data['front_vertex_count'],axis=1)];rear=f[~np.all(f<data['front_vertex_count'],axis=1)];s=math.sin(math.radians(35));c=math.cos(math.radians(35));projected=np.column_stack((v[:,0],-v[:,1]*s-v[:,2]*c));centroids=projected[front].mean(1);lo=projected[:data['front_vertex_count']].min(0);hi=projected[:data['front_vertex_count']].max(0);grid=np.array([[x+.5,y+.5] for y in range(math.floor(lo[1]),math.ceil(hi[1]),2) for x in range(math.floor(lo[0]),math.ceil(hi[0]),2)]);samples=np.concatenate((centroids,grid))
 def firstheight(triangles):
  nearest=np.full(len(samples),-np.inf)
  for face in triangles:
   t=projected[face];aa=t[1]-t[0];bb=t[2]-t[0];det=aa[0]*bb[1]-aa[1]*bb[0]
   if abs(det)<1e-10:continue
   k=samples-t[0];u=(k[:,0]*bb[1]-k[:,1]*bb[0])/det;w=(aa[0]*k[:,1]-aa[1]*k[:,0])/det;inside=(u>=-1e-9)&(w>=-1e-9)&(u+w<=1+1e-9);zz=v[face[0],2]+u*(v[face[1],2]-v[face[0],2])+w*(v[face[2],2]-v[face[0],2]);nearest[inside]=np.maximum(nearest[inside],zz[inside])
  return nearest
 fh,rh=firstheight(front),firstheight(rear);domain=np.isfinite(fh);bad=domain&(rh>fh+1e-6);audit={'status':'PASS_FRONT_REAR_SAMPLED_FIRSTHITS' if not bad.any() else 'HOLD_REAR_INTRUSIONS','samples_with_front':int(domain.sum()),'front_centroids':len(centroids),'grid_spacing_source_pixels':2,'rear_firsthit_intrusions':int(bad.sum()),'bad_source_xy':samples[bad].tolist(),'limit':'Source-camera sampling only, not all-pairs triangle selfintersection proof.'};(a.output/'front-rear-firsthits.json').write_text(json.dumps(audit,indent=2)+'\n')
 r={'status':'CPU_DIAGRAM_NOT_SAVED_RENDER','geometry_sha256':hashlib.sha256(a.geometry.read_bytes()).hexdigest(),'vertices':len(v),'triangles':len(f),'native_view_index':0,'limits':['Painter-sorted orthographic triangle diagrams only.','No material/sourceownership, intersections or saved model validation.']};(a.output/'receipt.json').write_text(json.dumps(r,indent=2)+'\n')
if __name__=='__main__':main()

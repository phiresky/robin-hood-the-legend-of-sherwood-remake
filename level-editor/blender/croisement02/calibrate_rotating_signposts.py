"""Bounded rigid sign silhouette calibration against native rotation samples."""
import json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import differential_evolution
from catalog import OUT


def main():
    base=OUT/'state-sign-candidate';data=json.loads((OUT/'state-target-evidence/manifest.json').read_text());frames=next(p for p in data['profiles'] if p['id']=='TG_Panel-12')['rows'][0]['frames'];indices=list(range(0,32,2));known=[]
    for i in indices:
        f=frames[i];im=Image.open(f['image']).convert('RGBA');a=np.asarray(im);yy=np.indices(a.shape[:2])[0]+f['offset'][1];mask=(a[:,:,3]>127)&~((a[:,:,:3].max(axis=2)<=12)&(yy>=0));canvas=Image.new('L',(80,80));canvas.paste(Image.fromarray(mask.astype('uint8')*255),(int(f['offset'][0])+40,int(f['offset'][1])+60));known.append(np.asarray(canvas)>0)
    outline=np.array([(-19,-36),(16,-36),(16,-32),(18,-32),(18,-31),(19,-31),(19,-27),(20,-27),(20,-24),(19,-24),(19,-18),(8,-18),(8,-17),(3,-17),(3,-18),(-19,-18),(-19,-22),(-20,-22),(-20,-30),(-19,-30)],float);S=math.sin(math.radians(35));C=math.cos(math.radians(35));z=(-outline[:,1]+S*1.4)/C;center=z.mean()
    def masks(p):
        width,height,zshift,board_y,thickness,postheight,postradius,phase=p
        results=[]
        for index in indices:
            theta=(13-index)*math.tau/32+phase;c,s=math.cos(theta),math.sin(theta)
            board=np.array([(x*width,y,(zz-center)*height+center+zshift) for y in (board_y-thickness/2,board_y+thickness/2) for x,zz in zip(outline[:,0],z)]);n=len(z)
            post=np.array([(x,y,zv) for zv in (0,postheight) for x,y in [(-postradius,-postradius),(postradius,-postradius),(postradius,postradius),(-postradius,postradius)]])
            im=Image.new('L',(80,80));draw=ImageDraw.Draw(im)
            for vertices,faces in [(board,[list(range(n)),list(range(n,n*2))]+[[i,(i+1)%n,(i+1)%n+n,i+n] for i in range(n)]),(post,[[0,1,2,3],[4,5,6,7],[0,1,5,4],[1,2,6,5],[2,3,7,6],[3,0,4,7]])]:
                x=vertices[:,0]*c-vertices[:,1]*s;y=vertices[:,0]*s+vertices[:,1]*c;points=np.stack((x+40,-S*y-C*vertices[:,2]+60),axis=1)
                for face in faces:draw.polygon([tuple(points[i]) for i in face],fill=255)
            results.append(np.asarray(im)>0)
        return results
    def loss(p):return np.mean([np.logical_xor(a,b).sum()/np.logical_or(a,b).sum() for a,b in zip(masks(p),known)])
    initial=[1,1,0,2.25,2.8,46,1.75,0]
    result=differential_evolution(loss,[(.9,1.15),(.85,1.25),(-3,3),(0,4),(1,3.5),(43,51),(1.2,2.2),(-.16,.16)],maxiter=32,popsize=6,seed=95,polish=False,workers=1)
    report=dict(status='diagnostic bounded rigid physical fit; visual review required',parameters=dict(zip(['board_width_scale','board_height_scale','board_z_shift','board_center_y','board_thickness','post_height','post_radius','phase_radians'],map(float,result.x))),initial_mean_iou=1-loss(initial),fitted_mean_iou=1-result.fun,iterations=result.nit,frame_samples=indices,scope='Fixed35degree camera and rigid yaw. No pose-specific deformation or photo-plane extrusion.')
    (base/'rigid-calibration.json').write_text(json.dumps(report,indent=2)+'\n');print(report)

if __name__=='__main__':main()

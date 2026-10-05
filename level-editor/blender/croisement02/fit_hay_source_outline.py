"""Read-only parameter fit of a closed hay mound to its native silhouette."""
import json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import minimize
from scipy.ndimage import label

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))
BOX=(860,930,1050,1110)


def geometry(parameters):
    xr,yr,crest,height,cx,cy=parameters;vertices=[(cx,-cy/SIN+30,height)];faces=[];rings=16;n=64
    for ring in range(1,rings+1):
        r=ring/rings;effective=crest+(1-crest)*r
        for j in range(n):
            angle=j*math.tau/n;uneven=1+.025*math.sin(5*angle)+.015*math.cos(9*angle)
            x=cx+xr*effective*math.cos(angle)*uneven
            y=-cy/SIN+yr*effective*math.sin(angle)*uneven+30*(1-r)
            z=height*(1-r**1.7)**1.5+1.1*math.sin(7*angle+r*8)*math.sin(math.pi*r)
            vertices.append((x,y,z))
    for j in range(n):faces.append((0,1+j,1+(j+1)%n))
    for ring in range(rings-1):
        a=1+ring*n;b=a+n
        for j in range(n):faces.append((a+j,b+j,b+(j+1)%n,a+(j+1)%n))
    faces.append(tuple(reversed(range(1+(rings-1)*n,1+rings*n))))
    return np.array(vertices),faces


def main():
    dest=OUT/'restart2-vegetation/hay-outline-research';dest.mkdir(exist_ok=True)
    cropped=np.asarray(Image.open(OUT/'baseline/masks/000124.png').convert('L'))>0
    target=np.zeros((BOX[3]-BOX[1],BOX[2]-BOX[0]),bool);target[966-BOX[1]:966-BOX[1]+cropped.shape[0],878-BOX[0]:878-BOX[0]+cropped.shape[1]]=cropped
    labels,n=label(target);sizes=np.bincount(labels.ravel());sizes[0]=0;mainmask=labels==sizes.argmax()
    def raster(parameters):
        points,faces=geometry(parameters);screen=np.column_stack((points[:,0]-BOX[0],-points[:,1]*SIN-points[:,2]*COS-BOX[1]));im=Image.new('L',(target.shape[1]*3,target.shape[0]*3));d=ImageDraw.Draw(im)
        for face in faces:d.polygon([tuple(screen[i]*3) for i in face],fill=255)
        return np.asarray(im)[1::3,1::3]>0
    def scores(mask):
        inside=int((mask&mainmask).sum());extra=int((mask&~mainmask).sum());missing=int((~mask&mainmask).sum());return dict(inside=inside,extra=extra,missing=missing,iou=inside/(inside+extra+missing))
    def loss(x):
        r=scores(raster(x));return r['missing']+1.3*r['extra']
    initial=np.array([58,55,0,60,956,1036.]);result=minimize(loss,initial,method='Powell',bounds=[(52,83),(45,100),(0,.16),(53,70),(946,963),(1027,1046)],options=dict(maxiter=8,maxfev=1400,xtol=.07,ftol=.001))
    fitted=raster(result.x);before=raster(initial);source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(BOX)
    overlay=np.array(source);overlay[mainmask&~fitted,:3]=(255,0,120);overlay[fitted&~mainmask,:3]=(0,170,255);Image.fromarray(overlay).resize((760,720),Image.Resampling.NEAREST).save(dest/'source-residual.png')
    Image.fromarray(mainmask.astype('uint8')*255).save(dest/'core-domain.png')
    report=dict(status='Private analytic silhouette hypothesis, not a saved model or approval',parameter_order=['x_radius','y_radius','crest_fraction','height','center_x','ground_center_source_y'],initial=initial.tolist(),fitted=result.x.tolist(),before=scores(before),after=scores(fitted),native_domain_pixels=int(target.sum()),core_domain_pixels=int(mainmask.sum()),disconnected_source_pixels=int((target&~mainmask).sum()),source_box=BOX,method='Closed rounded mound, native silhouette fit only; hidden depth and slope inferred. Original worker untouched.',limitations=['Core fit excludes disconnected straw flecks; these are not transferred to ground.','Requires actual8, original-camera material comparison and ground-contact review.'])
    (dest/'fit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'before':report['before'],'after':report['after'],'fitted':report['fitted']}))

if __name__=='__main__':main()

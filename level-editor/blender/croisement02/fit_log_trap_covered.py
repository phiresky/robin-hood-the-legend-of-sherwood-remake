"""Fit surveyed solid-cylinder endpoints to native covered log silhouettes."""
import json,math
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import minimize
from scipy.spatial import ConvexHull
from catalog import OUT
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
def main():
    root=OUT/'state-target-evidence';data=json.loads((root/'manifest.json').read_text());p=next(p for p in data['profiles'] if p['profile']=='Croisement02 - buches01_b1');frame=next(r for r in p['rows'] if r['action_id']==0)['frames'][0];alpha=np.array(Image.open(frame['image']))[:,:,3]>127;h,w=alpha.shape
    initial=np.array([(5,77,87,103,5.8),(9,64,103,96,6.1),(12,51,107,85,6.2),(16,39,114,77,6.2),(19,27,115,66,6.0),(22,15,111,54,6.1),(25,5,104,43,5.8)])
    angles=np.arange(16)*math.tau/16
    def raster(values):
        image=Image.new('L',(w,h));draw=ImageDraw.Draw(image)
        for ax,ay,bx,by,r in values.reshape(-1,5):
            tangent=np.array([bx-ax,-(by-ay)/SIN]);tangent/=np.linalg.norm(tangent);cross=np.array([-tangent[1],-tangent[0]*SIN]);ring=r*(np.cos(angles)[:,None]*cross+np.sin(angles)[:,None]*[0,-COS]);points=np.concatenate([ring+[ax,ay],ring+[bx,by]]);hull=ConvexHull(points);draw.polygon([tuple(v)for v in points[hull.vertices]],fill=255)
        return np.array(image)>0
    bounds=[]
    for row in initial:
        bounds.extend([(value-9,value+9)for value in row[:4]]+[(3,10)])
    def objective(values):
        actual=raster(values);intersection=(actual&alpha).sum();union=(actual|alpha).sum();regularizer=np.mean(((values.reshape(-1,5)-initial)/[9,9,9,9,4])**2)*.006
        return 1-intersection/union+regularizer
    result=minimize(objective,initial.ravel(),method='Powell',bounds=bounds,options=dict(maxiter=12,xtol=.08,ftol=1e-5,maxfev=16000));actual=raster(result.x);survey=[[*map(float,row),float(z)]for row,z in zip(result.x.reshape(-1,5),[12,23,36,49,61,73,85])]
    report=dict(method='Seven surveyed closed cylinders; bounded endpoint/radius fitting to own native wood alpha, no added reference source.',initial=initial.tolist(),survey=survey,fit_iou=float((actual&alpha).sum()/(actual|alpha).sum()),fit_native_coverage=float((actual&alpha).sum()/alpha.sum()),source_sha256=frame['image_sha256'],limitations='Native-pixel silhouette fit only; actual solid geometry, oblique depth and source material alignment require Blender review.')
    (root/'log-trap/covered-cylinder-fit.json').write_text(json.dumps(report,indent=2)+'\n');Image.fromarray(actual.astype('uint8')*255).save(root/'log-trap/covered-cylinder-fit.png');print(report['fit_iou'],report['fit_native_coverage'])
if __name__=='__main__':main()

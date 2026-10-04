"""Fit complete ellipsoidal hypotheses to partially observed initial rock caps."""
import json,math,hashlib
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import minimize
from catalog import OUT
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
def main():
    source=OUT/'state-target-evidence/rock-trap';path=source/'tick--01.png';alpha=np.array(Image.open(path))[:,:,3]>0;h,w=alpha.shape;initial=np.array([(116,27,13,13),(132,29,10,11),(143,37,8,11)],float)
    def raster(values):
        image=Image.new('L',(w,h));draw=ImageDraw.Draw(image)
        for x,y,rx,rz in values.reshape(-1,4):
            ry=math.sqrt((rx*SIN)**2+(rz*COS)**2);draw.ellipse((x-rx,y-ry,x+rx,y+ry),fill=255)
        return np.array(image)>0
    def objective(values):
        actual=raster(values);miss=(alpha&~actual).sum();extra=(actual&~alpha).sum();return (3*miss+extra*.35)/alpha.sum()+np.mean(((values.reshape(-1,4)-initial)/8)**2)*.02
    bounds=[]
    for x,y,rx,rz in initial:bounds.extend([(x-6,x+6),(y-7,y+7),(max(5,rx-5),rx+7),(max(5,rz-5),rz+7)])
    result=minimize(objective,initial.ravel(),method='Powell',bounds=bounds,options=dict(maxiter=10,xtol=.1,ftol=1e-5,maxfev=14000));actual=raster(result.x);report=dict(status='unapproved ellipsoid hypothesis, requires actual terrain and foliage review',source_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),survey=result.x.reshape(-1,4).tolist(),native_coverage=float((actual&alpha).sum()/alpha.sum()),iou=float((actual&alpha).sum()/(actual|alpha).sum()),limitations=['Ellipsoids remain complete; fitting cap alpha does not remove hidden lower/rear rock geometry.','Pixel-raster fit must be checked against actual mesh projection.','Three volume hypotheses do not prove exact boulder identities through native motion.']);(source/'covered-ellipsoid-fit.json').write_text(json.dumps(report,indent=2)+'\n');print(report)
if __name__=='__main__':main()

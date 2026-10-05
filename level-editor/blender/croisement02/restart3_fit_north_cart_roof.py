"""Fit a finite barrel canopy to surveyed native front and rear arch boundaries."""
import json,hashlib,math
from pathlib import Path
import numpy as np
from scipy.optimize import least_squares
from PIL import Image,ImageDraw
from catalog import OUT
SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))
FRONT=np.array([(106,44),(111,43),(119,44),(128,46),(138,50),(148,55),(157,61)],float)
REAR=np.array([(149,5),(156,4),(165,5),(176,8),(186,12),(195,18),(200,24)],float)


def curves(p):
    angle,length,width,rise,cx,cy=p;t=np.linspace(math.pi,0,257);across=width*np.cos(t)
    front=np.stack([cx+math.sin(angle)*across,cy+math.cos(angle)*SIN*across-rise*COS*np.sin(t)],axis=-1)
    return front,front+np.array([math.cos(angle)*length,-math.sin(angle)*SIN*length])


def main():
    def residual(p):
        a,b=curves(p)
        return np.concatenate([np.linalg.norm(points[:,None]-curve[None],axis=2).min(axis=1) for points,curve in [(FRONT,a),(REAR,b)]])
    result=least_squares(residual,[1.02,84,30,10,132,53],bounds=([.9,70,25,4,125,46],[1.15,100,37,18,140,62]),max_nfev=1500)
    dest=OUT/'restart3-north-cart/roof-fit-v1';dest.mkdir(exist_ok=False)
    source=OUT/'state-target-evidence/profiles/chariot03-10/action-160-direction-0-frame-000.png';im=Image.open(source).convert('RGBA');canvas=Image.new('RGBA',im.size,(35,35,35,255));canvas.alpha_composite(im);canvas=canvas.resize((848,832),Image.Resampling.NEAREST);draw=ImageDraw.Draw(canvas)
    for curve in curves(result.x):draw.line([tuple(p*4) for p in curve],fill=(0,220,250),width=2)
    for label,points in [('F',FRONT),('R',REAR)]:
        for index,(x,y) in enumerate(points):draw.ellipse((x*4-3,y*4-3,x*4+3,y*4+3),outline='red',width=2);draw.text((x*4+3,y*4),f'{label}{index}',fill='red')
    canvas.save(dest/'survey-fit.png')
    report=dict(source=str(source),source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),survey={'front_arch':FRONT.tolist(),'rear_arch':REAR.tolist(),'uncertainty_native_pixels':2},parameters=result.x.tolist(),parameter_names=['axis_angle','length','halfwidth','rise','front_eave_center_native_x','front_eave_center_native_y'],maximum_residual=float(max(residual(result.x))),rms_residual=float(np.sqrt(np.mean(residual(result.x)**2))),status='Source fit only; saved geometry/contact review required',limitations=['Manual native cloth edge survey with 1–2px uncertainty.','Unseen roof underside/thickness and structure inferred.','No translation or deformation animation approved.'])
    (dest/'fit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))

if __name__=='__main__':main()

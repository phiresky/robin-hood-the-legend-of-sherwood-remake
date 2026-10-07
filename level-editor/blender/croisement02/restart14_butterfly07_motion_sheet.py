"""CPU projection sheet; source frames and inferred motion explicitly separated."""
import json
import numpy as np
from PIL import Image,ImageDraw
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation,Slerp
from restart14_butterfly07_pose_fit import geometry
import restart14_butterfly_canopy22_audit as reader
B=reader.B;OUT=B/'butterfly07-body-axis-selected-v1'

def main():
    fitp=OUT/'fit.json';reportp=B/'butterfly07-body-axis-selected-motion-v1/report.json';fit=json.loads(fitp.read_text());rows={r['phase']:r for r in fit['rows']};report=json.loads(reportp.read_text());sheet=Image.new('RGB',(1250,960),'#252525');draw=ImageDraw.Draw(sheet)
    for row,k in enumerate([18,19,20,21,91,92]):
        a,b=rows[k],rows[k+1];p0,p1=np.array(a['parameters']),np.array(b['parameters']);rot=Slerp([0,1],Rotation.from_euler('xyz',[p0[:3],p1[:3]],degrees=True))
        for col,t in enumerate([0,.25,.5,.75,1]):
            p=(1-t)*p0+t*p1;p[:3]=rot([t])[0].as_euler('xyz',degrees=True);ox=col*250;oy=row*150;origin=np.array([ox+120,oy+80]);scale=9
            if t in [0,1]:
                frame=a if t==0 else b;im=Image.open(frame['source']['source']).convert('RGBA');arr=np.asarray(im);yy,xx=np.nonzero(arr[:,:,3]);center=np.array([xx.mean()+.5,yy.mean()+.5]);im=im.resize((im.width*scale,im.height*scale),Image.Resampling.NEAREST);sheet.paste(im,tuple(np.rint(origin-center*scale).astype(int)),im)
            label=f'Source{k if t==0 else k+1} + fit'if t in [0,1]else f'Inferred{k}+{t}'
            draw.text((ox+4,oy+3),label,fill='white');body,wings=geometry(p);parts={}
            if t==.5:parts=report['results'][f'selected:midpoint:{k}-{k+1}']['parts']
            for name,v,colr in [('body',body,'orange'),('wing0',wings[0],'#ff66cc'),('wing1',wings[1],'#22ddff')]:
                poly=v[:,:2]+p[5:7]
                if len(poly)>7:poly=poly[ConvexHull(poly).vertices]
                points=[tuple(origin+q*scale)for q in np.vstack([poly,poly[0]])];draw.line(points,fill='#ff3333'if name in parts else colr,width=2)
            if t==.5:draw.text((ox+4,oy+132),'Exact hit: '+','.join(parts)if parts else'Midpoint clear; sweep separate',fill='#ff7777'if parts else'#99ffbb')
            elif t in [.25,.75]:draw.text((ox+4,oy+132),'Graph sample audit separate',fill='#cccccc')
    draw.text((4,915),'Endpoints: native source image + inferred fixed anatomy. Intermediate frames: inferred rigid motion, no native image.',fill='white');draw.text((4,935),'Red parts: confirmed midpoint contacts. Fixed native anchor/height curve; body/wing dimensions and materials unchanged.',fill='white');sheet.save(OUT/'small-motion-candidate.png');(OUT/'motion-sheet-binding.json').write_text(json.dumps({'fit_sha256':reader.sha(fitp),'contacts_sha256':reader.sha(reportp),'sheet_sha256':reader.sha(OUT/'small-motion-candidate.png'),'recipe_sha256':reader.sha(__import__('pathlib').Path(__file__))},indent=2)+'\n')

if __name__=='__main__':main()

"""Fit explicitly inferred hanging bag volumes to annotated native endpoint outlines."""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from scipy.optimize import least_squares
from PIL import Image,ImageDraw
from catalog import OUT


def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()

# Source-image coordinates; the left waist behind the wooden piece is inferred.
SURVEYS={
 'piege01e':[(35,6),(40,14),(47,27),(50,39),(48,49),(41,57),(34,61),(27,59),(23,50),(22,41),(24,31),(28,22)],
 'piege01i':[(35,6),(43,15),(51,25),(56,35),(60,45),(59,50),(48,60),(43,65),(34,69),(26,68),(20,65),(15,60),(13,55),(12,50),(12,42),(14,32),(18,25),(23,20),(27,15),(31,10)],
 'piege03e':[(43,18),(48,29),(60,43),(63,58),(56,70),(48,80),(40,82),(31,76),(27,65),(29,54),(34,42),(40,29)],
 'piege03i':[(43,18),(53,25),(62,35),(66,45),(71,55),(74,65),(71,70),(62,80),(55,85),(50,90),(40,93),(28,90),(21,85),(17,80),(15,75),(14,70),(14,62),(15,52),(18,40),(24,35),(29,30),(33,25),(37,20)],
}
FRACTIONS=np.array([0,.1,.25,.4,.55,.7,.85,1.])
SIN=np.sin(np.deg2rad(35));COS=np.cos(np.deg2rad(35))


def vertices(parameters,ratio):
 cx,bottom,height,tilt,*radii=parameters;angles=np.arange(48)*2*np.pi/48;rows=[]
 for t,r in zip(FRACTIONS,[.15,*radii,.15]):
  for angle in angles:rows.append([cx+tilt*(t-.5)+r*np.cos(angle),-bottom/SIN+ratio*r*np.sin(angle),t*height])
 return np.array(rows)


def main():
 parser=argparse.ArgumentParser();parser.add_argument('--output',default='net-endpoint-volume-fit-v2');args=parser.parse_args()
 root=OUT/'net-state-source-review-v1';source=json.loads((root/'manifest.json').read_text());dest=OUT/args.output;dest.mkdir(exist_ok=True);assert not(dest/'manifest.json').exists();records=[];sheet=Image.new('RGB',(1200,840),'#444');draw=ImageDraw.Draw(sheet)
 directions=np.column_stack((np.cos(np.arange(96)*2*np.pi/96),np.sin(np.arange(96)*2*np.pi/96)))
 for family in ['piege01','piege03']:
  item=next(r for r in source['net_instances']if r['instance']['target']['profile_name'].endswith(family+'h'))
  for variant in 'ei':
   key=family+variant;patch=next(p for p in item['patches']if p['name'].endswith(key));frame=patch['states']['final']['frames'][0];path=OUT/'source-states'/frame['image'];outline=np.array(SURVEYS[key],float);target=(outline@directions.T).max(axis=0)+.5;cx=outline[:,0].mean();height=np.ptp(outline[:,1])/COS;bottom=outline[:,1].max();width=np.ptp(outline[:,0]);ratio=.55 if variant=='e' else 1.;radii=np.sin(FRACTIONS[1:-1]*np.pi)**.8*width/2;initial=np.r_[cx,bottom,height,0,radii]
   def residual(params):
    v=vertices(params,ratio);projected=np.column_stack((v[:,0],-v[:,1]*SIN-v[:,2]*COS));support=(projected@directions.T).max(axis=0)
    return np.r_[support-target,.08*np.diff(params[4:],n=2)]
   result=least_squares(residual,initial,bounds=(np.r_[cx-8,bottom-10,height*.6,-10,np.repeat(.1,6)],np.r_[cx+8,bottom+10,height*1.4,10,np.repeat(width*.8,6)]),max_nfev=500)
   assert result.success,(key,result.message);v=vertices(result.x,ratio);projected=np.column_stack((v[:,0],-v[:,1]*SIN-v[:,2]*COS));res=residual(result.x)[:96];image=Image.open(path).convert('RGBA');preview=Image.new('RGBA',image.size,'#444');preview.alpha_composite(image);preview=preview.resize((image.width*4,image.height*4),Image.Resampling.NEAREST);marks=ImageDraw.Draw(preview)
   marks.line([(x*4,y*4)for x,y in SURVEYS[key]]+[tuple(outline[0]*4)],fill=(255,50,220),width=2)
   for x,y in projected:marks.ellipse((x*4-1,y*4-1,x*4+1,y*4+1),fill=(40,230,255))
   name=key+'-outline-fit.png';preview.save(dest/name);n=len(records);x=n%2*600;y=n//2*420;sheet.paste(preview.convert('RGB'),(x,y+25));draw.text((x+4,y+4),f'{key}: fitted outline; hidden left waist inferred',fill='white')
   records.append(dict(family=family,variant=variant,patch_id=patch['id'],source=str(path),source_sha256=sha(path),bbox=frame['bbox'],outline_crop_coordinates=SURVEYS[key],manual_uncertainty_pixels=2,parameters=result.x.tolist(),ring_fractions=FRACTIONS.tolist(),depth_to_horizontal_radius=ratio,support_error_rms=float(np.sqrt(np.mean(res**2))),support_error_max=float(np.abs(res).max()),review_image=name,limitations=['Depth and axis orientation are inferred, not measured.','Left waist behind the wooden piece is completed from visible bag shape.','This volume describes bag appearance only; it does not contain identified human geometry.','Wooden piece, cords, attachments, initial rigging and all moving phases remain separate.']))
 sheet.save(dest/'outline-fit-sheet.png');(dest/'manifest.json').write_text(json.dumps(dict(status='Private volume fit; source trace and actual 3D review pending',source_manifest_sha256=sha(root/'manifest.json'),records=records),indent=2)+'\n');print([(r['family']+r['variant'],r['support_error_rms'],r['support_error_max'])for r in records])
if __name__=='__main__':main()

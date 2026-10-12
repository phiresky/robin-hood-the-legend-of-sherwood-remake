"""Assign endpoint source texels to one nearest saved-model surface owner."""
from pathlib import Path
import json,math,hashlib
import numpy as np
from PIL import Image
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
def ownership(objects,width=300,height=240,left=420,top=380):
    depth=np.full((height,width),-np.inf);owner=np.full((height,width),-1,dtype=np.int32)
    for index,record in enumerate(objects):
        vertices=np.asarray(record['vertices'],dtype=float)
        screen=np.column_stack((vertices[:,0]-left,-vertices[:,1]*SIN-vertices[:,2]*COS-top))
        distances=-vertices[:,1]*COS+vertices[:,2]*SIN
        for face in record['triangles']:
            a,b,c=screen[face];xmin=max(0,int(math.floor(min(a[0],b[0],c[0]))));xmax=min(width-1,int(math.ceil(max(a[0],b[0],c[0]))));ymin=max(0,int(math.floor(min(a[1],b[1],c[1]))));ymax=min(height-1,int(math.ceil(max(a[1],b[1],c[1]))))
            den=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
            if abs(den)<1e-10 or xmin>xmax or ymin>ymax:continue
            y,x=np.mgrid[ymin:ymax+1,xmin:xmax+1];x=x+.5;y=y+.5
            wa=((b[1]-c[1])*(x-c[0])+(c[0]-b[0])*(y-c[1]))/den
            wb=((c[1]-a[1])*(x-c[0])+(a[0]-c[0])*(y-c[1]))/den;wc=1-wa-wb
            valid=(wa>=-1e-9)&(wb>=-1e-9)&(wc>=-1e-9);z=wa*distances[face[0]]+wb*distances[face[1]]+wc*distances[face[2]]
            target=depth[ymin:ymax+1,xmin:xmax+1];replace=valid&(z>target+1e-8);target[replace]=z[replace];owner[ymin:ymax+1,xmin:xmax+1][replace]=index
    return owner,depth

def write_atlases(objects,source,output):
    output.mkdir(parents=True,exist_ok=False);rgba=np.array(Image.open(source).convert('RGBA'));owner,_=ownership(objects);assert owner.shape==rgba.shape[:2]
    # Packed blue shadow markers are metadata and cannot become solid log texels.
    rgb=rgba[:,:,:3].astype(np.uint16);packed=((rgb[:,:,0]>>3)<<11)|((rgb[:,:,1]>>2)<<5)|(rgb[:,:,2]>>3)
    known=(rgba[:,:,3]>=128)&(packed!=0x001f)&(packed!=0x07c0);records=[]
    palette=np.zeros_like(rgba);palette[:,:,3]=255
    for i,ob in enumerate(objects):
        tex=rgba.copy();mask=known&(owner==i);tex[:,:,3]=np.where(mask,rgba[:,:,3],0);file=output/f'owner-{i:02}.png';Image.fromarray(tex).save(file);palette[owner==i,:3]=[(i*73+67)%255,(i*113+37)%255,(i*41+131)%255]
        records.append({'object':ob['name'],'path':str(file),'known_pixels':int(mask.sum()),'sha256':hashlib.sha256(file.read_bytes()).hexdigest()})
    Image.fromarray(palette).save(output/'nearest-owner.png');outside=known&(owner<0);neutral=(owner>=0)&~known
    diagnostic=np.zeros_like(rgba);diagnostic[known]=rgba[known];diagnostic[outside]=[255,20,20,255];diagnostic[neutral]=[40,130,255,255];Image.fromarray(diagnostic).save(output/'coverage.png')
    report={'status':'source-projection ownership only; visual review required','source_sha256':hashlib.sha256(Path(source).read_bytes()).hexdigest(),'owners':records,'source_opaque_pixels':int(known.sum()),'source_pixels_outside_geometry':int(outside.sum()),'solid_pixels_without_source':int(neutral.sum()),'source_pixels_assigned_once':sum(x['known_pixels'] for x in records),'limitations':['Raster ownership uses native pixel centers and saved mesh triangles.','Unknown surfaces remain opaque neutral geometry; source alpha never cuts holes through solids.','Coverage counts include small source debris absent from the endpoint solids.','This proves source ownership, not physical animation identity or terrain contact.']}
    assert report['source_pixels_assigned_once']+report['source_pixels_outside_geometry']==report['source_opaque_pixels']
    (output/'report.json').write_text(json.dumps(report,indent=2)+'\n');return report

"""Recess complete inferred logs to source-supported silhouette bounds, without alpha clipping."""
import json,math,hashlib
import numpy as np
from PIL import Image,ImageDraw,ImageFilter
from scipy.spatial import ConvexHull
from catalog import OUT


def main():
    base=OUT/'restart2-state/log-triangular-pile-v4';fit=json.loads((base/'fit.json').read_text());native=Image.open(OUT/'state-target-evidence/log-trap/tick--01.png').convert('RGBA');expected=np.array(native)[:,:,3]>0;allowed=np.array(Image.fromarray(expected.astype(np.uint8)*255).filter(ImageFilter.MaxFilter(3)))>0;S,C=math.sin(math.radians(35)),math.cos(math.radians(35));u=np.array(fit['axis']);v=np.array(fit['cross_axis_away_from_camera']);w=np.array([0,0,1]);records=[];limits=[]
    def projection(points):return np.c_[points[:,0]-390,-S*points[:,1]-C*points[:,2]-441]
    def raster(points):
        q=projection(points);hull=q[ConvexHull(q).vertices];image=Image.new('1',native.size);ImageDraw.Draw(image).polygon([tuple(x)for x in hull],fill=1);return np.array(image)
    for index,old in enumerate(fit['records']):
        row=dict(old);a,b=np.array(row['start']),np.array(row['end']);center=(a+b)/2;radial=np.array([row['radius']*(v*math.cos(t)+w*math.sin(t))for t in np.linspace(0,2*math.pi,48,endpoint=False)])
        if row['observed_index'] is None:
            ts=np.arange(-100,101,dtype=float);good=[]
            for t in ts:
                mask=raster(center+u*t+radial);good.append(mask.sum()>0 and not np.any(mask&~allowed))
            runs=[];start=None
            for i,valid in enumerate(good+[False]):
                if valid and start is None:start=i
                if not valid and start is not None:runs.append((start,i-1));start=None
            assert runs,f'No inferred support interval {index}'
            run=max(runs,key=lambda r:r[1]-r[0]);lo,hi=ts[run[0]],ts[run[1]];assert hi-lo>=35,(index,hi-lo);row['start']=(center+u*lo).tolist();row['end']=(center+u*hi).tolist();limits.append(dict(index=index,layer=row['layer'],column=row['column'],original_length=float(np.linalg.norm(b-a)),bounded_length=float(hi-lo),axial_interval=[float(np.dot(row['start'],u)),float(np.dot(row['end'],u))]))
        records.append(row)
    body=np.zeros_like(expected)
    for row in records:
        radial=np.array([row['radius']*(v*math.cos(t)+w*math.sin(t))for t in np.linspace(0,2*math.pi,48,endpoint=False)]);body|=raster(np.concatenate([np.array(row['start'])+radial,np.array(row['end'])+radial]))
    supports=[]
    for upper in records:
        if upper['layer']==0:continue
        parents=[next(r for r in records if r['layer']==upper['layer']-1 and r['column']==j)for j in [upper['column'],upper['column']+1]];center=(np.array(upper['start'])+upper['end'])/2;t=float(center@u);support_ranges=[]
        for lower in parents:
            lo=max(float(np.dot(lower['start'],u)),float(np.dot(upper['start'],u)));hi=min(float(np.dot(lower['end'],u)),float(np.dot(upper['end'],u)));assert hi>lo;support_ranges.append([lo,hi])
        stable=sum(r[0]for r in support_ranges)/2<=t<=sum(r[1]for r in support_ranges)/2;supports.append(dict(layer=upper['layer'],column=upper['column'],center_within_two_contact_support_polygon=stable,axial_support_ranges=support_ranges));assert stable,(upper['layer'],upper['column'],t,support_ranges)
    dest=OUT/'restart2-state/log-triangular-pile-v5';dest.mkdir(exist_ok=False);report={**fit,'records':records,'status':'Private complete triangular pile with source-constrained inferred axial extents; exact solid audit and visual review pending','source_constraint_parent_sha256':hashlib.sha256((base/'fit.json').read_bytes()).hexdigest(),'inferred_axial_bounds':limits,'longitudinal_support':supports,'source_silhouette':dict(expected=int(expected.sum()),covered=int((body&expected).sum()),missing=int((expected&~body).sum()),excess=int((body&~expected).sum()),allowed_edge_margin=1)};(dest/'fit.json').write_text(json.dumps(report,indent=2)+'\n');rgba=np.array(native).copy();rgba[expected&~body]=[255,20,20,255];rgba[body&~expected]=[30,160,255,255];Image.fromarray(rgba).resize((768,711),Image.Resampling.NEAREST).save(dest/'native-silhouette-diagnostic.png');print(report['source_silhouette'],[round(r['bounded_length'])for r in limits])
if __name__=='__main__':main()

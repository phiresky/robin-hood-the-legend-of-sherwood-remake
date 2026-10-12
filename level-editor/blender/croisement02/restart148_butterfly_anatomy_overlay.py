"""Inspect source-frame silhouette ownership before committing to a saved render."""
import argparse,json,importlib.util,hashlib
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull
from PIL import Image
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

def main():
 p=argparse.ArgumentParser();p.add_argument('--fit',type=Path,required=True);p.add_argument('--geometry',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads(a.fit.read_text());spec=importlib.util.spec_from_file_location('anatomy',a.geometry);geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo);sha=lambda x:hashlib.sha256(x.read_bytes()).hexdigest();images=[]
 for start in range(0,len(fit['rows']),25):
  fig,axes=plt.subplots(5,5,figsize=(12,12),facecolor='#252525')
  for ax,row in zip(axes.flat,fit['rows'][start:start+25]):
   src=Path(row['source']['source']);assert sha(src)==row['source']['sha256'];rgba=np.asarray(Image.open(src).convert('RGBA'));yy,xx=np.nonzero(rgba[:,:,3]);shift=np.array([xx.mean()+.5,yy.mean()+.5])+row['parameters'][5:7];ax.set_facecolor('#252525');ax.imshow(rgba,interpolation='nearest');_,wings=geo.geometry(row['parameters'])
   for points in geo.body_components(row['parameters']):
    xy=points[:,:2];xy=xy[ConvexHull(xy).vertices]+shift-.5;xy=np.vstack([xy,xy[0]]);ax.plot(xy[:,0],xy[:,1],color='white',linewidth=.7)
   for points,color in zip(wings,['cyan','dodgerblue','magenta','orange']):
    xy=points[:,:2]+shift-.5;xy=np.vstack([xy,xy[0]]);ax.plot(xy[:,0],xy[:,1],color=color,linewidth=.7)
   h,w=rgba.shape[:2];ax.set_xlim(-2,w+1);ax.set_ylim(h+1,-2);ax.set_xticks([]);ax.set_yticks([]);ax.set_title(f"{row['phase']:02}: missed {row['missing']}, extra {row['extra']}",fontsize=8,color='white')
  for ax in list(axes.flat)[len(fit['rows'][start:start+25]):]:ax.axis('off')
  fig.tight_layout();dest=a.output/f'overlay-{start:02}.png';fig.savefig(dest,dpi=100);plt.close(fig);images.append({'path':str(dest),'sha256':sha(dest)})
 (a.output/'report.json').write_text(json.dumps({'status':'DIAGNOSTIC_SOURCE_GEOMETRY_OVERLAY_NOT_SAVED_MODEL_REVIEW','fit_sha256':sha(a.fit),'geometry_sha256':sha(a.geometry),'images':images,'body':'White outlines are individual body surfaces. Colored outlines are physical wing regions.','limits':['Mathematical projected geometry only; source appearance and rendered depth ordering need actual saved-model review.','No source pixel residual is waived by this diagnostic.']},indent=2)+'\n')
if __name__=='__main__':main()

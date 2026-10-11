"""Inspect proposed fixed body proportions against selected own-source poses."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import argparse,json
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import restart64_butterfly07_source_ownership as own
from restart21_butterfly07_geometry_v2 import geometry,BODY,HINGES,WING

a=argparse.ArgumentParser();a.add_argument('--fit',type=Path,required=True);a.add_argument('--probe',type=Path,required=True);a.add_argument('--output',type=Path,required=True);args=a.parse_args();args.output.mkdir(exist_ok=False)
fit=json.loads(args.fit.read_text());report=json.loads(args.probe.read_text());assert own.sha(args.fit)==report['fit_sha256'];scale=fit['fixed_uniform_scale']
def geom(p):
 b,w=geometry(p);return b*scale,[x*scale for x in w]
own.geometry=geom;own.HINGES=np.array(HINGES)*scale
rows=sorted(fit['rows'],key=lambda r:r['missing']+3*r['bright_missing'],reverse=True)[:16]
fig,axes=plt.subplots(4,4,figsize=(12,12),facecolor='#202020')
for ax,row in zip(axes.flat,rows):
 c=own.prepare(row);rot=Rotation.from_euler('xyz',row['parameters'][:3],degrees=True);ax.set_facecolor('#202020');ax.imshow(c['rgba'],interpolation='nearest')
 for modified,color,style in [(False,'red',':'),(True,'white','-')]:
  b=BODY.copy()
  if modified:b[:,0]*=report['width_factor'];b[:,1]*=report['length_factor'];b[:,1]+=report['local_y_offset']
  xy=rot.apply(b*scale)[:,:2]+c['shift']-.5;poly=xy[ConvexHull(xy).vertices];poly=np.vstack([poly,poly[0]]);ax.plot(poly[:,0],poly[:,1],style,color=color,lw=1)
 for m in c['maps']:
  xy=(WING*scale)[:,:2]@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift']-.5;xy=np.vstack([xy,xy[0]]);ax.plot(xy[:,0],xy[:,1],color='cyan',lw=.6)
 h,w=c['rgba'].shape[:2];ax.set_xlim(-3,w+3);ax.set_ylim(h+3,-3);ax.set_xticks([]);ax.set_yticks([]);ax.set_title(str(row['phase']),color='white')
fig.tight_layout();fig.savefig(args.output/'body-overlay.png',dpi=140);plt.close(fig)
(args.output/'pins.json').write_text(json.dumps(dict(fit=own.sha(args.fit),probe=own.sha(args.probe),recipe=own.sha(Path(__file__))),indent=2)+'\n')

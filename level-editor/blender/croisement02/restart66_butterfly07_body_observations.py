"""Recover only geometrically visible body observations from the butterfly's own frames."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
from restart64_butterfly07_source_ownership import prepare,WING,ROOT,sha
BASE=ROOT/'level-editor/work/croisement02-refinement';FIT=BASE/'restart64-butterfly07-pose-refinement-v1/fit.json';OUT=BASE/'restart66-butterfly07-body-observations-v1'
def main():
 assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());w,h=64,32;iy,ix=np.mgrid[:h,:w];a=(iy.ravel()+.5)/h*np.pi;t=(ix.ravel()+.5)/w*2*np.pi;points=np.c_[.5*np.sin(a)*np.cos(t),2.7*np.cos(a),.5*np.sin(a)*np.sin(t)];normals=points/np.array([.5**2,2.7**2,.5**2]);normals/=np.linalg.norm(normals,axis=1)[:,None];scores=np.zeros(h*w);colors=np.full((h*w,4),255,np.uint8);colors[:,:3]=112;phases=np.full(h*w,-1,int);source_xy=np.full((h*w,2),-1,int)
 for row in fit['rows']:
  c=prepare(row);rotation=Rotation.from_euler('xyz',row['parameters'][:3],degrees=True).as_matrix();v=points@rotation.T;n=normals@rotation.T;xy=v[:,:2]+c['shift'];x=np.floor(xy[:,0]).astype(int);y=np.floor(xy[:,1]).astype(int);img=c['rgba'];valid=(n[:,2]>0)&(x>=0)&(x<img.shape[1])&(y>=0)&(y<img.shape[0])
  for m in c['maps']:
   uv=(xy-c['shift']-m['offset'][:2])@np.linalg.inv(m['matrix'][:2,:2]).T;z=uv@m['matrix'][2,:2]+m['offset'][2];valid&=~(Polygon(WING[:,:2]).contains_points(uv)&(z>=v[:,2]))
  ids=np.flatnonzero(valid);ids=ids[img[y[ids],x[ids],3]>0];ids=ids[n[ids,2]>scores[ids]];scores[ids]=n[ids,2];colors[ids,:3]=img[y[ids],x[ids],:3];phases[ids]=row['phase'];source_xy[ids]=np.c_[x[ids],y[ids]]
 known=phases>=0;Image.fromarray(colors.reshape(h,w,4)).save(OUT/'body.png');Image.fromarray((known.reshape(h,w)*255).astype(np.uint8)).save(OUT/'known.png');np.savez(OUT/'provenance.npz',phase=phases.reshape(h,w),source_xy=source_xy.reshape(h,w,2));report=dict(status='PRIVATE_BODY_OBSERVATION_PROPOSAL',fit_sha256=sha(FIT),source_frames_checked=99,known_texels=int(known.sum()),unknown_texels=int((~known).sum()),image_sha256=sha(OUT/'body.png'),texture_coordinates='u=azimuth/(2pi);v=polar/pi, long axis+Y',limits=['Visible ellipsoid surface points exclude both nearer wings.','Exact RGB copied from best-facing own source frame, not synthesis or averaging.','Body is inferred and less than one source pixel wide in many frames; source pixels can contain antialias mixtures.','Ellipsoid parameter sampling differs slightly from tessellated mesh surface; verify rendered appearance before approval.','Unobserved texels remain gray.']);(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(report)
if __name__=='__main__':main()

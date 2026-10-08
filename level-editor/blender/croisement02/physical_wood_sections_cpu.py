"""Private physical stem-volume preview from native inscribed discs, CPU only.

Only the permitted cottage/moat tube references inform the construction style.
No existing model is opened, changed, selected, or approved by this experiment.
"""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter,binary_fill_holes,label
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from mpl_toolkits.mplot3d.art3d import Poly3DCollection
from prepare_wood_field_integration import source,CONFIG,OUT,STUDY
from continuous_wood_field import SIN,COS,RAY
from smooth_wood_field import constrained_rim

def reference_bindings():
 base=OUT.parent/'leicester-refinement/round-1/texture-review/approved-evidence'
 result=[]
 for name,digest in [('leicester-southeast-cottage-tree','9a8974c11e28d00c9729745d71fdc25d00385fb48a002e8b51833e5809fd5312'),('leicester-moat-bank-tree','6a18f955edc12e45fc5f05892022419f07aa527b9960c16a651645ce8c593392')]:
  for filename in ['model.blend','solid.png']:
   path=base/name/digest/filename
   result.append(dict(asset=name,path=str(path),sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
 return result

def medial_discs(body,origin):
 field,_=constrained_rim(body)
 fig,ax=plt.subplots();contours=ax.contour(np.arange(field.shape[1])-8+origin[0]+.5,np.arange(field.shape[0])-8+origin[1]+.5,field,levels=[0]).allsegs[0];plt.close(fig)
 contour=max(contours,key=len);start=contour[:-1];delta=np.diff(contour,axis=0);length2=np.sum(delta*delta,axis=1)
 y,x=np.where(body);centers=np.column_stack((x+origin[0]+.5,y+origin[1]+.5));_,nearest=cKDTree(start).query(centers,k=6)
 candidates=np.mod(np.concatenate((nearest,nearest-1),axis=1),len(start));a=start[candidates];d=delta[candidates];t=np.clip(np.sum((centers[:,None,:]-a)*d,axis=2)/np.maximum(length2[candidates],1e-20),0,1);r=np.linalg.norm(centers[:,None,:]-a-t[:,:,None]*d,axis=2).min(axis=1)
 selected=[]
 for i in np.argsort(-r):
  if selected and np.any(np.linalg.norm(centers[selected]-centers[i],axis=1)+r[i]<=r[selected]+1e-6):continue
  selected.append(int(i))
 return centers[selected],r[selected],contour,centers

def tetra_surface(values,axes):
 nz,ny,nx=values.shape;vertices=[];faces=[];cache={};flat=values.ravel()
 offsets=np.array([0,1,nx,nx+1,nx*ny,nx*ny+1,nx*ny+nx,nx*ny+nx+1]);tetrahedra=[(0,1,3,7),(0,3,2,7),(0,2,6,7),(0,6,4,7),(0,4,5,7),(0,5,1,7)]
 cells=np.stack([values[z:z+nz-1,y:y+ny-1,x:x+nx-1] for z,y,x in [(0,0,0),(0,0,1),(0,1,0),(0,1,1),(1,0,0),(1,0,1),(1,1,0),(1,1,1)]])
 active=np.argwhere((cells.min(axis=0)<0)&(cells.max(axis=0)>0));origin=np.array([a[0] for a in axes]);step=axes[0][1]-axes[0][0]
 def point(i):return origin+np.array([i%nx,(i//nx)%ny,i//(nx*ny)])*step
 def cross(a,b):
  key=(min(a,b),max(a,b))
  if key not in cache:
   t=flat[a]/(flat[a]-flat[b]);cache[key]=len(vertices);vertices.append(point(a)+t*(point(b)-point(a)))
  return cache[key]
 for z,y,x in active:
  cube=(z*ny+y)*nx+x+offsets
  for local in tetrahedra:
   ids=cube[list(local)];negative=ids[flat[ids]<0];positive=ids[flat[ids]>=0]
   if not len(negative) or not len(positive):continue
   outward=np.mean([point(i) for i in positive],axis=0)-np.mean([point(i) for i in negative],axis=0)
   if len(negative)==1:polygons=[tuple(cross(int(negative[0]),int(i)) for i in positive)]
   elif len(positive)==1:polygons=[tuple(cross(int(positive[0]),int(i)) for i in negative)]
   else:
    a,b=map(int,negative);c,d=map(int,positive);q=[cross(a,c),cross(a,d),cross(b,d),cross(b,c)];polygons=[(q[0],q[1],q[2]),(q[0],q[2],q[3])]
   for face in polygons:
    a,b,c=[vertices[i] for i in face]
    if np.dot(np.cross(b-a,c-a),outward)<0:face=tuple(reversed(face))
    faces.append(face)
 return np.array(vertices),np.array(faces,int)

def projection_coverage(vertices, faces, body, origin):
    projected = np.column_stack((vertices[:,0], -vertices[:,1]*SIN-vertices[:,2]*COS))
    covered = np.zeros(body.shape, bool)
    for face in faces:
        p = projected[face]-np.asarray(origin)-.5
        lo = np.maximum(0, np.ceil(p.min(axis=0)).astype(int))
        hi = np.minimum(np.array(body.shape[::-1])-1, np.floor(p.max(axis=0)).astype(int))
        if np.any(lo>hi):continue
        vx, vy = p[1]-p[0];wx, wy = p[2]-p[0];det=vx*wy-vy*wx
        if abs(det)<1e-12:continue
        yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];px=xx-p[0,0];py=yy-p[0,1]
        t=(px*wy-py*wx)/det;u=(vx*py-vy*px)/det
        z=vertices[face[0],2]+t*(vertices[face[1],2]-vertices[face[0],2])+u*(vertices[face[2],2]-vertices[face[0],2])
        inside=(t>=-1e-9)&(u>=-1e-9)&(t+u<=1+1e-9)&(z>.0003)
        covered[lo[1]:hi[1]+1,lo[0]:hi[0]+1]|=inside
    return dict(body_centers=int(body.sum()),covered_body_centers=int((body&covered).sum()),missing_body_centers=int((body&~covered).sum()),source_ground_coverage=float((body&covered).sum()/body.sum()),silhouette_iou=float((body&covered).sum()/(body|covered).sum()),missing_coordinates=[[int(x+origin[0]),int(y+origin[1])] for y,x in np.argwhere(body&~covered)],scope='CPU exact projected triangle barycentrics at source centers; pinned ground plane approximation, no neighbouring owners')

def safe_surface(volume, axes, voxel):
    target=.15*voxel
    nearby=np.unique(volume[(volume>target-.006*voxel)&(volume<target+.006*voxel)])
    mids=(nearby[1:]+nearby[:-1])/2;gaps=np.diff(nearby)
    candidates=mids[np.argsort(-gaps)[:12]] if len(mids) else [target]
    failures=[]
    for level in candidates:
        vertices,faces=tetra_surface(volume-level,axes)
        q=vertices.astype(np.float32);area=np.linalg.norm(np.cross(q[faces[:,1]]-q[faces[:,0]],q[faces[:,2]]-q[faces[:,0]]),axis=1)/2
        bad=int((area<1e-9).sum());failures.append(dict(level=float(level),degenerate_faces=bad,minimum_face_area=float(area.min())))
        if not bad:return vertices,faces,dict(level=float(level),target=target,attempts=failures)
    raise ValueError('No numerically stable CPU isosurface: '+str(failures))

def main():
 p=argparse.ArgumentParser();p.add_argument('--tree',type=int,choices=[32,38],default=32);p.add_argument('--voxel',type=float,default=1.5);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 if a.output.exists():raise FileExistsError(a.output)
 box=CONFIG[a.tree][2];observed,sourcepath=source(a.tree,box);labels,count=label(observed);sizes=np.bincount(labels.ravel());sizes[0]=0;body=binary_fill_holes(labels==int(np.argmax(sizes)));coverage=dict(observed_pixels=int(observed.sum()),main_body_observed_pixels=int((body&observed).sum()),unresolved_disconnected_pixels=int((observed&~body).sum()));centers,radii,contour,native=medial_discs(body,box[:2]);print('Inscribed round sections:',len(radii),flush=True);ground=next(r['ground_y'] for r in json.load(open(OUT/'forest-v4-sources/manifest.json')) if r['mask']==a.tree)
 world=np.column_stack((centers[:,0],np.full(len(centers),-ground/SIN),(ground-centers[:,1])/COS))
 # Raise source silhouettes out of the floor along their own camera rays.
 lift=np.maximum(0,radii*COS+.25-world[:,2]);world+=lift[:,None]/SIN*RAY
 lower=np.min(world-radii[:,None],axis=0)-3*a.voxel;upper=np.max(world+radii[:,None],axis=0)+3*a.voxel;axes=[np.arange(lo,hi+a.voxel,a.voxel) for lo,hi in zip(lower,upper)]
 volume=np.full(tuple(len(axis) for axis in axes[::-1]),4*a.voxel)
 for center,radius in zip(world,radii):
  starts=[max(0,int(np.searchsorted(axis,c-radius-3*a.voxel))) for axis,c in zip(axes,center)];ends=[min(len(axis),int(np.searchsorted(axis,c+radius+3*a.voxel))+1) for axis,c in zip(axes,center)];local=[axis[i:j] for axis,i,j in zip(axes,starts,ends)];xx,yy,zz=np.meshgrid(*local,indexing='ij');distance=np.sqrt((xx-center[0])**2+(yy-center[1])**2+(zz-center[2])**2)-radius;region=tuple(slice(i,j) for i,j in zip(starts[::-1],ends[::-1]));volume[region]=np.minimum(volume[region],distance.transpose(2,1,0))
 print('Physical volume sampled:',volume.shape,flush=True)
 # A small preview-only grid relaxation suppresses subvoxel sphere seams.
 # It is NOT a saved native-coverage claim; final guards remain mandatory.
 volume=gaussian_filter(volume,.4)
 vertices,faces,precision=safe_surface(volume,axes,a.voxel);triangles=vertices[faces];print('CPU solid mesh:',len(vertices),len(faces),flush=True)
 sections=[]
 for z in [20,40,60,80,100]:
  points=[]
  for i,j in [(0,1),(1,2),(2,0)]:
   lo=triangles[:,i];hi=triangles[:,j];hit=(lo[:,2]-z)*(hi[:,2]-z)<0;lo=lo[hit];hi=hi[hit];points.extend(lo+(hi-lo)*((z-lo[:,2])/(hi[:,2]-lo[:,2]))[:,None])
  if points:
   extent=np.ptp(points,axis=0);sections.append(dict(z=z,width=float(extent[0]),depth=float(extent[1]),ratio=float(extent[1]/extent[0])))
 a.output.mkdir(parents=True);np.savez_compressed(a.output/'preview-mesh.npz',vertices=vertices,faces=faces);np.savez_compressed(a.output/'disc-plan.npz',source_centers=centers,radii=radii,world_centers=world,contour=contour)
 normals=np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0]);normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);light=np.array([-.4,-.6,.7]);light/=np.linalg.norm(light);shade=.35+.6*np.maximum(0,normals@light);colors=np.column_stack((shade,shade,shade,np.ones(len(shade))))
 fig=plt.figure(figsize=(12,9));middle=vertices.mean(axis=0);width=np.ptp(vertices,axis=0).max()/2
 for n,az in enumerate([-90,0,90,180],1):
  ax=fig.add_subplot(2,2,n,projection='3d');ax.add_collection3d(Poly3DCollection(triangles,facecolors=colors,edgecolors='none'));ax.set(xlim=(middle[0]-width,middle[0]+width),ylim=(middle[1]-width,middle[1]+width),zlim=(middle[2]-width,middle[2]+width));ax.set_box_aspect((1,1,1));ax.view_init(20,az);ax.set_proj_type('ortho');ax.set_axis_off();ax.set_title(f'CPU physical volume / azimuth {az}')
 fig.suptitle('Private mass proposal only — retained limb joins and saved source/ground guards unfinished');fig.tight_layout();fig.savefig(a.output/'solid-preview.png',dpi=130);plt.close(fig)
 theoretical=np.max(radii[None,:]-np.linalg.norm(native[:,None,:]-centers[None,:,:],axis=2),axis=1)
 report=dict(status='CPU volume proposal only; no Blender model or canonical changes',tree=a.tree,source_mask=str(sourcepath),source_sha256=hashlib.sha256(sourcepath.read_bytes()).hexdigest(),coverage=coverage,medial_discs=len(radii),theoretical_body_centers_covered=int((theoretical>=-1e-6).sum()),body_centers=len(native),voxel=a.voxel,vertices=len(vertices),faces=len(faces),sections=sections,source_ground=projection_coverage(vertices,faces,body,box[:2]),precision=precision,permitted_references=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'],limitations=['The preview mesh uses a small grid relaxation; theoretical disc coverage is not saved mesh source coverage.','Retained upper limbs are not attached in this preview.','Private inferred front depth changed; native rays and source ownership remain the eventual constraints.'])
 (a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
if __name__=='__main__':main()

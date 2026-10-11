"""Reconstruct upper collars as a smooth volume, independent of old triangulation.

Private anatomy exploration only. The sampled lower volume is not an exact
bank-contact replacement; a passing upper surface would need stitching to the
retained lower original geometry before review.
"""
import gc,hashlib,json,shutil
from pathlib import Path
import numpy as np
from scipy.ndimage import distance_transform_edt,gaussian_filter
from vtkmodules.vtkCommonCore import VTK_UNSIGNED_CHAR,VTK_FLOAT
from vtkmodules.vtkCommonDataModel import vtkImageData
from vtkmodules.vtkImagingStencil import vtkPolyDataToImageStencil,vtkImageStencil
from vtkmodules.vtkFiltersCore import vtkFlyingEdges3D,vtkTriangleFilter
from vtkmodules.util.numpy_support import vtk_to_numpy,numpy_to_vtk
from restart2_tree08_fork_kernel import poly
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
O=R/'tree08-implicit-collar-v1';assert not O.exists()
assert shutil.disk_usage(R).free>=10*1024**3
assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
source=R/'tree08-connected-tip-cleanup-v2/candidate.npz';a=np.load(source);v,f=a['vertices'],a['faces']
origin=np.floor(v.min(0))-8;dims=(np.ceil(v.max(0)+8-origin)+1).astype(int)
image=vtkImageData();image.SetOrigin(*origin);image.SetSpacing(1,1,1);image.SetDimensions(*dims);image.AllocateScalars(VTK_UNSIGNED_CHAR,1);vtk_to_numpy(image.GetPointData().GetScalars()).fill(1)
stencil=vtkPolyDataToImageStencil();stencil.SetInputData(poly(v,f));stencil.SetOutputOrigin(*origin);stencil.SetOutputSpacing(1,1,1);stencil.SetOutputWholeExtent(image.GetExtent());stencil.Update()
fill=vtkImageStencil();fill.SetInputData(image);fill.SetStencilConnection(stencil.GetOutputPort());fill.SetBackgroundValue(0);fill.Update()
inside=vtk_to_numpy(fill.GetOutput().GetPointData().GetScalars()).reshape(tuple(dims[::-1])).astype(bool)
print('GRID',dims.tolist(),'INSIDE',int(inside.sum()),flush=True)
distance=distance_transform_edt(inside).astype(np.float32);distance-=distance_transform_edt(~inside).astype(np.float32);del inside;gc.collect()
filtered=gaussian_filter(distance,sigma=3.,mode='nearest')
s,c=np.sin(np.radians(35)),np.cos(np.radians(35));z=np.arange(dims[2])+origin[2];y=np.arange(dims[1])+origin[1];x=np.arange(dims[0])+origin[0]
native_y=-s*y[None,:]-c*z[:,None]
def smooth(t):
 t=np.clip(t,0,1);return t*t*(3-2*t)
weight=smooth((315-native_y)/35)[:,:,None]*smooth((x-460)/45)[None,None,:]*smooth((700-x)/45)[None,None,:]
field=(distance+(filtered-distance)*weight).astype(np.float32);del distance,filtered,weight;gc.collect()
volume=vtkImageData();volume.SetDimensions(*dims);volume.SetOrigin(*origin);volume.SetSpacing(1,1,1);volume.GetPointData().SetScalars(numpy_to_vtk(field.ravel(),deep=True,array_type=VTK_FLOAT))
contour=vtkFlyingEdges3D();contour.SetInputData(volume);contour.SetValue(0,0);contour.ComputeNormalsOff();contour.Update();mesh=contour.GetOutput()
vertices=vtk_to_numpy(mesh.GetPoints().GetData()).astype(float);cells=vtk_to_numpy(mesh.GetPolys().GetData()).reshape(-1,4);assert (cells[:,0]==3).all();faces=cells[:,1:].copy()
check=audit(vertices,faces);O.mkdir();np.savez_compressed(O/'candidate.npz',vertices=vertices,faces=faces)
report=dict(status='PRIVATE_IMPLICIT_ANATOMY_TRIAL_NOT_CONTACT_OR_SOURCE_VALIDATED',source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),spacing=1.,gaussian_sigma=3.,vertices=len(vertices),faces=len(faces),topology=check,limits='Voxelized lower surface is diagnostic only. A usable upper shape requires exact retained-root stitching, all source pixel coverage/RGBA, topology and strict intersections. No approval or synthesis.')
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)

"""Restore only genuinely lost source-pixel support on an inferred fairing trial."""
import hashlib,json,sys
from pathlib import Path
import numpy as np
from PIL import Image
from vtkmodules.vtkCommonCore import reference
from vtkmodules.vtkCommonDataModel import vtkGenericCell,vtkStaticCellLocator
from restart2_tree08_fork_kernel import poly
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2'
prior=R/sys.argv[1];out=R/sys.argv[2]
assert prior.resolve().parent==out.resolve().parent==R.resolve() and not out.exists()
a=np.load(prior/'candidate.npz');v,f=a['before_vertices'],a['faces'];delta=a['vertices']-v
s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0.,-c,s]);down=np.array([0.,-s,-c])
pixels=np.argwhere(np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0)
positions=np.array([np.array([x+331.5,0,0])+down*(y+11.5) for y,x in pixels])
def trace(vertices):
    loc=vtkStaticCellLocator();loc.SetDataSet(poly(vertices,f));loc.BuildLocator();cell=vtkGenericCell();faces=[]
    for p in positions:
        t,sub,face=reference(0.),reference(0),reference(0)
        found=loc.IntersectWithLine(p+ray*2000,p-ray*2000,1e-9,t,[0.,0.,0.],[0.,0.,0.],sub,face,cell)
        faces.append(int(face) if found else -1)
    return np.array(faces)
source_faces=trace(v);assert (source_faces>=0).all()
base=v[f];normal=np.cross(base[:,1]-base[:,0],base[:,2]-base[:,0]);area2=np.einsum('ij,ij->i',normal,normal)
weight=np.ones(len(v));pin=np.zeros(len(v),bool);origin=np.array([552.,-672.,235.]);passes=[]
for iteration in range(20):
    for oriented in range(80):
        vertices=(v+delta*weight[:,None]-origin).astype(np.float32).astype(float)+origin
        vertices[(weight<1e-3)|~np.any(delta,axis=1)]=v[(weight<1e-3)|~np.any(delta,axis=1)]
        t=vertices[f];n=np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0]);ratio=np.einsum('ij,ij->i',normal,n)/area2
        bad=ratio<.05
        if not bad.any():break
        weight[np.unique(f[bad])]*=.5
    assert not bad.any()
    lost=trace(vertices)<0;passes.append(dict(iteration=iteration,lost_pixels=int(lost.sum()),frozen_vertices=int(pin.sum())))
    print(passes[-1],flush=True)
    if not lost.any():break
    pin[np.unique(f[source_faces[lost]])]=True
    pin[np.unique(f[pin[f].any(axis=1)])]=True
    weight[pin]=0;halo=pin.copy()
    for w in [.25,.5,.75]:
        expanded=halo.copy();expanded[np.unique(f[halo[f].any(axis=1)])]=True
        weight[expanded&~halo]=np.minimum(weight[expanded&~halo],w);halo=expanded
assert not lost.any()
check=audit(vertices,f);assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=f,before_vertices=v)
report=dict(status='PRIVATE_COVERAGE_RESTORED_FAIRING_REQUIRES_STRICT_AND_VISUAL_REVIEW',candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),prior_sha256=hashlib.sha256((prior/'candidate.npz').read_bytes()).hexdigest(),source_pixels=len(pixels),lost_pixels=0,topology=check,coverage_passes=passes,changed_vertices=int(np.any(vertices!=v,axis=1).sum()),limits='Coverage only. Source-facing inferred depth may change. Saved source RGBA and strict intersections remain unproved.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)

"""Smooth inferred source-ray depth without moving projected source geometry."""
import hashlib
import json
from pathlib import Path
import sys
import numpy as np
from scipy.sparse import coo_matrix,diags
from restart2_tree08_local_fork import audit

R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
packet=R/sys.argv[1]
assert packet.resolve().parent==R.resolve()
out=R/'tree08-projected-depth-fairing-v1'
assert not out.exists()
data=np.load(packet/'candidate.npz');v,f=data['vertices'],data['faces']
before_check=audit(v,f)
assert not any(before_check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
s,c=np.sin(np.radians(35)),np.cos(np.radians(35))
ray,down=np.array([0.,-c,s]),np.array([0.,-s,-c])
native_y=v@down
edges=np.unique(np.sort(np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]),axis=1),axis=0)
row=np.r_[edges[:,0],edges[:,1]];col=np.r_[edges[:,1],edges[:,0]]
adjacency=coo_matrix((np.ones(len(row)),(row,col)),shape=(len(v),len(v))).tocsr()
degree=np.asarray(adjacency.sum(axis=1)).ravel()
average=diags(1/degree)@adjacency
initial=v@ray;depth=initial.copy();active=native_y<315
for _ in range(80):
    for gain in [.45,-.46]:
        depth[active]+=(gain*(average@depth-depth))[active]
        depth=np.clip(depth,initial-20.,initial+20.)
        depth[~active]=initial[~active]
delta=depth-initial
triangles=v[f]
normals=np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0])
area2=np.einsum('ij,ij->i',normals,normals)
weight=np.ones(len(v));weight[~active]=0
origin=np.array([552.,-672.,235.])
for step in range(80):
    vertices=v+(delta*weight)[:,None]*ray
    vertices=(vertices-origin).astype(np.float32).astype(float)+origin
    vertices[weight<1e-3]=v[weight<1e-3]
    t=vertices[f];n=np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0])
    ratio=np.einsum('ij,ij->i',n,normals)/area2
    bad=ratio<.05
    if not bad.any():break
    weight[np.unique(f[bad])]*=.5
assert not bad.any()
check=audit(vertices,f)
assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
projection_error=float(np.abs((vertices-v)@down).max())
assert projection_error<.0001
assert np.array_equal(vertices[:,0],v[:,0])
assert np.array_equal(vertices[~active],v[~active])
out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=f,before_vertices=v)
report=dict(status='PRIVATE_INFERRED_DEPTH_TRIAL_NEEDS_SOURCE_RGB_AND_INTERSECTION_CHECKS',parent_sha256=hashlib.sha256((packet/'candidate.npz').read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),maximum_projected_coordinate_error=projection_error,maximum_inferred_depth_change=float(np.abs((vertices-v)@ray).max()),lower_root_positions_exact=True,topology=check,orientation_damping_steps=step,minimum_oriented_area_ratio=float(ratio.min()),scope='Artwork fixes projected position/color/ownership, not hidden depth. Only upper source-ray depth changes; terrain contact geometry stays fixed. Saved source projection, RGB, visibility and strict intersections remain required.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)

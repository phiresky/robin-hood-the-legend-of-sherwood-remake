"""Constrained CPU rear-curvature experiment; no Blender or selection writes.

Every trial remains on its native source ray. A line search enforces triangle
orientation and float32 area, rather than repairing a fold after accepting it.
"""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np
from scipy.sparse import coo_matrix, diags, eye
from continuous_wood_field import RAY, SIN, COS


def fair(vertices, faces, iterations=250):
    tri=vertices[faces]
    normals=np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0])
    vn=np.zeros_like(vertices)
    for i in range(3):np.add.at(vn,faces[:,i],normals)
    vn/=np.linalg.norm(vn,axis=1)[:,None]
    facing=vn@RAY
    weights=np.clip((-facing-.2)/.5,0,1)*np.clip((vertices[:,2]-12)/12,0,1)*np.clip((120-vertices[:,2])/15,0,1)
    edges=np.unique(np.sort(np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]]),axis=1),axis=0)
    adj=coo_matrix((np.ones(2*len(edges)),(np.r_[edges[:,0],edges[:,1]],np.r_[edges[:,1],edges[:,0]])),shape=(len(vertices),len(vertices))).tocsr()
    lap=eye(len(vertices))-diags(1/np.asarray(adj.sum(axis=1)).ravel())@adj
    original=vertices@RAY;delta=np.zeros(len(vertices));trials=[]
    normal2=np.sum(normals*normals,axis=1)
    def energy(d):
        curvature=lap@(original+d)
        return float(curvature@curvature+.1*(d@d))
    initial=energy(delta)
    for iteration in range(iterations):
        gradient=2*(lap.T@(lap@(original+delta))+.1*delta)
        # Zero weighted mean avoids a systematic contraction of the rear.
        gradient-=float(np.sum(gradient*weights)/max(weights.sum(),1))
        direction=-gradient*weights
        scale=.15;accepted=False;before=energy(delta)
        for attempt in range(22):
            candidate=np.clip(delta+scale*direction,-1.5,1.5)
            new=vertices+candidate[:,None]*RAY
            t=new[faces];nn=np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0])
            orientation=np.sum(nn*normals,axis=1)
            q=new.astype(np.float32)[faces]
            area=np.linalg.norm(np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0]),axis=1)/2
            after=energy(candidate)
            if np.all(orientation>=.1*normal2) and np.all(area>=1e-9) and after<before:
                accepted=True;delta=candidate;break
            scale*=.5
        trials.append(dict(iteration=iteration,accepted=accepted,step=scale,energy=after,minimum_normal_fraction=float(np.min(orientation/normal2))))
        if not accepted or np.max(abs(scale*direction))<1e-8:break
    new=vertices+delta[:,None]*RAY
    projection=lambda v:np.column_stack((v[:,0],-v[:,1]*SIN-v[:,2]*COS))
    final_tri=new[faces];final_normals=np.cross(final_tri[:,1]-final_tri[:,0],final_tri[:,2]-final_tri[:,0])
    report=dict(initial_energy=initial,final_energy=energy(delta),iterations=len(trials),maximum_world_displacement=float(abs(delta).max()),maximum_projected_displacement=float(abs(projection(new)-projection(vertices)).max()),source_front_vertices_moved=int(((facing>=-.2)&(abs(delta)>1e-9)).sum()),contact_vertices_moved=int(((vertices[:,2]<=12)&(abs(delta)>1e-9)).sum()),upper_attachment_vertices_moved=int(((vertices[:,2]>=120)&(abs(delta)>1e-9)).sum()),reversed_triangles=int((np.sum(normals*final_normals,axis=1)<=0).sum()),minimum_normal_fraction=float(np.min(np.sum(normals*final_normals,axis=1)/normal2)),mean_rear_depth_change=float(delta[weights>0].mean()),trials=trials)
    return new,report


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--input',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    if args.output.exists():raise FileExistsError(args.output)
    mesh=np.load(args.input,allow_pickle=False);vertices,report=fair(mesh['vertices'],mesh['faces'])
    args.output.mkdir(parents=True);np.savez_compressed(args.output/'diagnostic.npz',vertices=vertices,faces=mesh['faces'])
    report.update(status='CPU constrained curvature diagnostic; no saved model or readiness claim',input=str(args.input.resolve()),input_sha256=hashlib.sha256(args.input.read_bytes()).hexdigest(),recipe_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k!='trials'},indent=2))


if __name__=='__main__':main()

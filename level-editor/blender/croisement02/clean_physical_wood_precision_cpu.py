"""Merge sub-millipixel wood slivers, then recheck topology and source centers."""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.ndimage import label,binary_fill_holes
from PIL import Image
from physical_wood_sections_cpu import projection_coverage
from prepare_wood_field_integration import source,CONFIG


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--input',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--merge-distance',type=float,default=.0005)
    args=parser.parse_args()
    if args.output.exists():raise FileExistsError(args.output)
    if not 0<args.merge_distance<=.001:raise ValueError('This cleanup is limited to sub-millipixel edges')
    mesh=np.load(args.input,allow_pickle=False);vertices,faces=mesh['vertices'],mesh['faces']
    pairs=cKDTree(vertices).query_pairs(args.merge_distance,output_type='ndarray')
    parent=np.arange(len(vertices))
    def root(i):
        while parent[i]!=i:parent[i]=parent[parent[i]];i=parent[i]
        return i
    for a,b in pairs:
        a,b=root(int(a)),root(int(b))
        if a!=b:parent[max(a,b)]=min(a,b)
    mapping=np.array([root(i)for i in range(len(vertices))])
    drift=float(np.linalg.norm(vertices[mapping]-vertices,axis=1).max())
    if drift>args.merge_distance*1.01:raise ValueError('Transitive cluster exceeds merge bound')
    triangles=mapping[faces];keep=(triangles[:,0]!=triangles[:,1])&(triangles[:,0]!=triangles[:,2])&(triangles[:,1]!=triangles[:,2]);triangles=triangles[keep]
    if len(np.unique(np.sort(triangles,axis=1),axis=0))!=len(triangles):raise ValueError('Cleanup created duplicate faces')
    edges=np.sort(np.concatenate([triangles[:,[0,1]],triangles[:,[1,2]],triangles[:,[2,0]]]),axis=1)
    _,counts=np.unique(edges,axis=0,return_counts=True)
    if np.any(counts!=2):raise ValueError('Cleanup opened the surface or made nonmanifold edges')
    used=np.unique(triangles);remap=np.full(len(vertices),-1);remap[used]=np.arange(len(used));new=vertices[used];triangles=remap[triangles]
    report=json.loads((args.input.parent/'report.json').read_text());box=tuple(report.get('source_box',CONFIG[report['tree']][2]));observed,path=source(report['tree'],box)
    if hashlib.sha256(path.read_bytes()).hexdigest()!=report['source_sha256']:raise ValueError('Source mask changed')
    construction=observed.copy();guide=report.get('construction_mask')
    if guide:
        p=Path(guide['path'])
        if hashlib.sha256(p.read_bytes()).hexdigest()!=guide['sha256']:raise ValueError('Construction guide changed')
        construction=np.asarray(Image.open(p).convert('L'))[box[1]:box[3],box[0]:box[2]]>0
    regions,_=label(construction,np.ones((3,3)) if report.get('construction_connectivity')==8 else None);sizes=np.bincount(regions.ravel());sizes[0]=0;body=binary_fill_holes(regions==sizes.argmax())
    observed_audit=projection_coverage(new,triangles,observed,box[:2]);body_audit=projection_coverage(new,triangles,body,box[:2])
    if observed_audit['missing_body_centers'] or body_audit['missing_body_centers']:raise ValueError('Precision cleanup lost source/construction centers')
    args.output.mkdir(parents=True);np.savez_compressed(args.output/'preview-mesh.npz',vertices=new,faces=triangles)
    report.update(vertices=len(new),faces=len(triangles),observed_source_ground=observed_audit,source_ground=body_audit,precision_cleanup={'input':str(args.input.resolve()),'input_sha256':hashlib.sha256(args.input.read_bytes()).hexdigest(),'merge_distance':args.merge_distance,'maximum_world_displacement':drift,'vertices_removed':len(vertices)-len(new),'collapsed_faces_removed':int((~keep).sum()),'closed_edge_incidence':True,'blender_guard_pending':True})
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['precision_cleanup'],indent=2))


if __name__=='__main__':main()

"""Merge fractional-pixel wood slivers, then recheck topology and source centers."""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.sparse import coo_matrix
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
    if not 0<args.merge_distance<=.01:raise ValueError('This cleanup is limited to one hundredth of a source pixel')
    mesh=np.load(args.input,allow_pickle=False);vertices,faces=mesh['vertices'],mesh['faces']
    pairs=np.unique(np.sort(np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]]),axis=1),axis=0)
    pairs=pairs[np.linalg.norm(vertices[pairs[:,0]]-vertices[pairs[:,1]],axis=1)<=args.merge_distance]
    adjacency=coo_matrix((np.ones(faces.size),(faces.ravel(),np.repeat(np.arange(len(faces)),3))),shape=(len(vertices),len(faces))).tocsr()
    initial=vertices[faces]
    initial_normals=np.cross(initial[:,1]-initial[:,0],initial[:,2]-initial[:,0])
    parent=np.arange(len(vertices))
    members={int(i):[int(i)]for i in np.unique(pairs)}
    representative={int(i):int(i)for i in np.unique(pairs)}
    skipped_pairs=0
    def root(i):
        while parent[i]!=i:parent[i]=parent[parent[i]];i=parent[i]
        return i
    lengths=np.linalg.norm(vertices[pairs[:,0]]-vertices[pairs[:,1]],axis=1)
    for a,b in pairs[np.argsort(lengths,kind='stable')]:
        a,b=root(int(a)),root(int(b))
        if a==b:continue
        group=np.array(sorted(members[a]+members[b]))
        points=vertices[group]
        radii=np.linalg.norm(points[:,None,:]-points[None,:,:],axis=2).max(axis=1)
        affected=np.unique(np.concatenate([adjacency.indices[adjacency.indptr[i]:adjacency.indptr[i+1]]for i in group]))
        current=np.array([representative.get(root(int(i)),root(int(i)))for i in faces[affected].ravel()]).reshape(-1,3)
        chosen=None
        for candidate in np.argsort(radii,kind='stable'):
            if radii[candidate]>args.merge_distance:break
            trial=current.copy();trial[np.isin(trial,[representative[a],representative[b]])]=int(group[candidate])
            keep=(trial[:,0]!=trial[:,1])&(trial[:,0]!=trial[:,2])&(trial[:,1]!=trial[:,2])
            q=vertices[trial[keep]].astype(np.float32)
            normals=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
            if np.any(np.linalg.norm(normals,axis=1)/2<1e-9):continue
            if np.any(np.sum(normals*initial_normals[affected[keep]],axis=1)<=0):continue
            chosen=int(candidate);break
        if chosen is None:
            skipped_pairs+=1
            continue
        first,last=min(a,b),max(a,b)
        parent[last]=first;members[first]=group.tolist();representative[first]=int(group[chosen])
        del members[last];del representative[last]
    mapping=np.array([representative.get(root(i),root(i))for i in range(len(vertices))])
    drift=float(np.linalg.norm(vertices[mapping]-vertices,axis=1).max())
    if drift>args.merge_distance*1.01:raise ValueError('Transitive cluster exceeds merge bound')
    triangles=mapping[faces];keep=(triangles[:,0]!=triangles[:,1])&(triangles[:,0]!=triangles[:,2])&(triangles[:,1]!=triangles[:,2]);triangles=triangles[keep]
    if len(np.unique(np.sort(triangles,axis=1),axis=0))!=len(triangles):raise ValueError('Cleanup created duplicate faces')
    edges=np.sort(np.concatenate([triangles[:,[0,1]],triangles[:,[1,2]],triangles[:,[2,0]]]),axis=1)
    _,counts=np.unique(edges,axis=0,return_counts=True)
    if np.any(counts!=2):raise ValueError('Cleanup opened the surface or made nonmanifold edges')
    used=np.unique(triangles);remap=np.full(len(vertices),-1);remap[used]=np.arange(len(used));new=vertices[used];triangles=remap[triangles]
    q=new.astype(np.float32)[triangles];normals=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
    if np.any(np.linalg.norm(normals,axis=1)/2<1e-9) or np.any(np.sum(normals*initial_normals[keep],axis=1)<=0):raise ValueError('Cleanup folded or collapsed a retained triangle')
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
    report.update(vertices=len(new),faces=len(triangles),observed_source_ground=observed_audit,source_ground=body_audit,precision_cleanup={'input':str(args.input.resolve()),'input_sha256':hashlib.sha256(args.input.read_bytes()).hexdigest(),'merge_distance':args.merge_distance,'maximum_world_displacement':drift,'representative_method':'Existing vertex with smallest maximum cluster distance; merges exceeding displacement or orientation bounds are skipped','skipped_pairs':skipped_pairs,'vertices_removed':len(vertices)-len(new),'collapsed_faces_removed':int((~keep).sum()),'closed_edge_incidence':True,'retained_triangle_orientation_preserved':True,'blender_guard_pending':True})
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['precision_cleanup'],indent=2))


if __name__=='__main__':main()

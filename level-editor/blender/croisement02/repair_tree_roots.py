"""Join traced wood and place inferred roots against the ground plane."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from tree_geometry import wood_geometry,replace_mesh,RAY
from bark_materials import fill
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import validate,modified
from audit_candidates import audit
from render_tree import render_workspace


def joined_paths(paths):
    # Skeleton splits leave short gaps at branch nodes and occluded roots.
    # Connect components by shortest centreline distance; these joins are inferred.
    components=[list(p) for p in paths];bridges=[]
    while len(components)>1:
        best=None
        for i,a in enumerate(components):
            aa=np.asarray(a)
            for j in range(i+1,len(components)):
                bb=np.asarray(components[j]);dist=np.linalg.norm(aa[:,None,:2]-bb[None,:,:2],axis=2)
                ia,ib=np.unravel_index(dist.argmin(),dist.shape)
                value=(float(dist[ia,ib]),i,j,ia,ib)
                if best is None or value[0]<best[0]:best=value
        distance,i,j,ia,ib=best
        if distance>40:raise ValueError('Wood fragments need separate ownership review')
        a,b=components[i][ia],components[j][ib]
        if distance>0:
            radius=max(1.,min(a[2],b[2]))
            bridges.append([[a[0],a[1],radius],[b[0],b[1],radius]])
        components[i]+=components.pop(j)
    return paths+bridges,len(bridges)


def repair(row):
    w=OUT/'forest-v4-round-1/assets'/row['asset_id'];receipt=w/'inspection/root-revision.json'
    acquire()
    if receipt.exists() and json.loads(receipt.read_text()).get('algorithm')!='union-before-ground-v2':
        previous=receipt.with_name('root-revision-v1.json')
        if previous.exists():raise ValueError('Previous revision already archived')
        receipt.rename(previous)
    if not receipt.exists():
        bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0;validate(w)
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name]
        wood=[o for o in objects if o.get('projection_component')!='crown']
        reports=[];bridges=0
        traces=next(r['paths'] for r in json.loads((OUT/'wood-traces.json').read_text()) if r['mask']==row['mask'])
        level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
        centers=[]
        for target in wood:
            pts=level['sight_obstacles'][int(target['source_node'].split('-')[-1])]['points']
            centers.append((np.mean([p['x'] for p in pts]),np.mean([p['y']-(p['z_top']+p['z_bottom'])/2 for p in pts])))
        assigned={o.name:[] for o in wood}
        for path in traces:
            center=np.mean(np.asarray(path)[:,:2],axis=0)
            winner=int(np.argmin(np.sum((np.asarray(centers)-center)**2,axis=1)))
            assigned[wood[winner].name].append(path)
        for target in wood:
            if len(wood)==1:
                paths,bridges=joined_paths(traces);verts,faces=wood_geometry(paths,row['ground_y'])
            else:
                # Reproduce the existing canonical assignment from source traces;
                # do not join different native parts into a single receiver.
                if not assigned[target.name]:raise ValueError('Native part has no trace')
                paths=assigned[target.name]
                if row['mask']==6:
                    paths,joined=joined_paths(paths);bridges+=joined
                verts,faces=wood_geometry(paths,row['ground_y'])
            points=np.asarray(verts)
            mesh=bpy.data.meshes.new('Joined wood temporary');mesh.from_pydata(points.tolist(),[],faces);mesh.update()
            obj=bpy.data.objects.new('Joined wood temporary',mesh);bpy.context.scene.collection.objects.link(obj)
            bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
            mesh.remesh_voxel_size=.8;mesh.use_remesh_preserve_volume=True;bpy.ops.object.voxel_remesh()
            bm=bmesh.new();bm.from_mesh(obj.data)
            bmesh.ops.smooth_vert(bm,verts=list(bm.verts),factor=.25,use_axis_x=True,use_axis_y=True,use_axis_z=True)
            bm.to_mesh(obj.data);bm.free()
            points=np.asarray([tuple(v.co) for v in obj.data.vertices]);z=points[:,2].copy()
            height=.5*(z+np.sqrt(z*z+36))
            points+=(height-z)[:,None]*np.asarray(RAY)[None,:]/RAY.z
            result=replace_mesh(target,points.tolist(),[tuple(p.vertices) for p in obj.data.polygons],materials=list(target.data.materials))
            bpy.data.objects.remove(obj,do_unlink=True)
            if result['nonmanifold_edges']:raise ValueError('Joined wood must be closed')
            for face in target.data.polygons:face.use_smooth=True
            reports.append(dict(source_node=target['source_node'],**result))
        before=sha(w/'model.blend');modified(w);bark=fill(w,objects,row['mask']);validate(w)
        bpy.ops.wm.save_as_mainfile(filepath=str(w/'model.blend'))
        report=json.loads((w/'inspection/refinement.json').read_text());report['wood']=reports;report['bark']=bark;report['model_sha256']=sha(w/'model.blend')
        report['limitations'].append('Root junctions and ground contact are inferred; short skeleton gaps are joined and low roots bend along the source ray.')
        write_json(w/'inspection/refinement.json',report);audit(w)
        write_json(receipt,dict(before_model_sha256=before,model_sha256=report['model_sha256'],inferred_bridges=bridges,voxel_size=.8,algorithm='union-before-ground-v2',geometry=reports))
    elif json.loads(receipt.read_text())['model_sha256']!=sha(w/'model.blend'):raise ValueError('Root revision changed')
    evidence=[w/'inspection/actual-materials/evidence.json',w/'inspection/source-coverage/report.json']
    if not all(p.exists() and json.loads(p.read_text()).get('model_sha256')==sha(w/'model.blend') for p in evidence):
        render_workspace(w,256,release_slot=False)
    release()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--masks',nargs='+',type=int,required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    latest={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    rows=json.loads((OUT/'forest-v4-sources/manifest.json').read_text());selected=[r for r in rows if r['mask'] in args.masks]
    if {r['mask'] for r in selected}!=set(args.masks):raise ValueError('Unknown mask')
    for r in selected:
        if latest.get(r['asset_id'],{}).get('decision')=='approved':raise ValueError('Approved geometry is frozen')
    try:
        for r in selected:repair(r)
    finally:release()

if __name__=='__main__':main()

"""Check complete rock endpoint solids against the immutable bank surface."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from audit_log_endpoint_contact import planar_triangle,overlap,sha


def main():
    base=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve() if '--candidate' in sys.argv else OUT/'rock-trap-state-candidate-v8'
    report=json.loads((base/'manifest.json').read_text())
    bank=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank'
    assert sha(base/'worker.blend')==report['model_sha256']
    assert sha(bank/'model.blend')==report['bank_model_sha256']
    audit=json.loads((bank/'inspection/saved-model-audit.json').read_text())
    names=[r['object']for r in audit['objects']if r['source_node']in[f'building-{i:03d}'for i in range(5)]]
    bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'))
    # The candidate already contains these exact receiver objects.
    banks=[bpy.data.objects[name]for name in names]
    bpy.context.view_layer.update()
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False)as(src,dst):dst.objects=list(names)
    for obj in dst.objects:bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.update()
    transform_proof=[]
    for name,saved,source in zip(names,banks,dst.objects):
        error=max(abs(saved.matrix_world[r][c]-source.matrix_world[r][c])for r in range(4)for c in range(4))
        assert error<1e-5,(name,'saved receiver transform changed',error)
        assert len(saved.data.vertices)==len(source.data.vertices)
        vertex_error=max((a.co-b.co).length for a,b in zip(saved.data.vertices,source.data.vertices))
        assert vertex_error<1e-5,(name,'saved receiver vertex changed',vertex_error)
        transform_proof.append(dict(object=name,world_matrix=[list(row)for row in saved.matrix_world],maximum_matrix_error=error,maximum_local_vertex_error=vertex_error,parent_is_none=saved.parent is None))
    planes=[]
    for obj in banks:
        obj.data.calc_loop_triangles()
        for triangle in obj.data.loop_triangles:
            plane=planar_triangle([obj.matrix_world@obj.data.vertices[i].co for i in triangle.vertices])
            if plane:planes.append(plane)
    bounds=np.array([[min(p.x for p in points),min(p.y for p in points),max(p.x for p in points),max(p.y for p in points)]for points,_,_ in planes])
    records=[]
    for obj in sorted((o for o in bpy.data.objects if o.get('state_endpoint')),key=lambda o:o.name):
        obj.data.calc_loop_triangles();minimum=min((obj.matrix_world@v.co).z for v in obj.data.vertices);worst=None
        for triangle in obj.data.loop_triangles:
            plane=planar_triangle([obj.matrix_world@obj.data.vertices[i].co for i in triangle.vertices])
            if plane is None:continue
            points,normal,offset=plane
            low=np.min(np.array(points)[:,:2],axis=0);high=np.max(np.array(points)[:,:2],axis=0)
            candidates=np.where((bounds[:,0]<=high[0])&(bounds[:,1]<=high[1])&(bounds[:,2]>=low[0])&(bounds[:,3]>=low[1]))[0]
            for index in candidates:
                bank_points,bank_normal,bank_offset=planes[index];intersection=overlap(points,bank_points)
                area=abs(sum(a[0]*b[1]-b[0]*a[1]for a,b in zip(intersection,intersection[1:]+intersection[:1])))/2
                if area<1e-5:continue
                for x,y in intersection:
                    rock_z=(offset-normal[0]*x-normal[1]*y)/normal[2];bank_z=(bank_offset-bank_normal[0]*x-bank_normal[1]*y)/bank_normal[2]
                    if rock_z-bank_z<minimum:minimum=rock_z-bank_z;worst=dict(x=x,y=y,rock_z=rock_z,bank_z=bank_z)
        row=dict(object=obj.name,state=obj['state_endpoint'],minimum_clearance=float(minimum),worst=worst);records.append(row);print(row,flush=True)
    (base/'contact-audit.json').write_text(json.dumps(dict(status='HOLD'if any(r['minimum_clearance']<-.05 or r['minimum_clearance']>1.05 for r in records)else 'surface clearance pass; silhouette and balance still require review',model_sha256=report['model_sha256'],bank_model_sha256=report['bank_model_sha256'],saved_reopened_receiver_proof=transform_proof,projected_overlap_area_tolerance=1e-5,records=records,limitations=['Checks bank height surfaces and ground plane; does not prove stable balance, rock-to-rock contacts or motion.']),indent=2)+'\n')


if __name__=='__main__':main()

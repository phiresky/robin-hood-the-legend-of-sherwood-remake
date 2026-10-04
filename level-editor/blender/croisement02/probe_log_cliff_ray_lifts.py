"""Probe rigid source-ray translations against narrow terrain corner contacts."""
import json
import sys
from pathlib import Path
import bpy
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import COS,SIN
from audit_log_endpoint_contact import planar_triangle,overlap,sha


def main():
    base=OUT/'log-trap-state-candidate-v9'
    report=json.loads((base/'dense-contact-audit.json').read_text())
    assert sha(base/'worker.blend')==report['model_sha256']
    bank=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank'
    assert sha(bank/'model.blend')==report['bank_model_sha256']
    audit=json.loads((bank/'inspection/saved-model-audit.json').read_text())
    names=[r['object']for r in audit['objects']if r['source_node']in[f'building-{i:03d}'for i in range(5)]]
    bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'))
    logs=[bpy.data.objects[f'applied log {i:02d}']for i in(4,5)]
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False)as(src,dst):dst.objects=names
    for obj in dst.objects:bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.update()
    bank_triangles=[]
    for obj in dst.objects:
        obj.data.calc_loop_triangles()
        for triangle in obj.data.loop_triangles:
            plane=planar_triangle([obj.matrix_world@obj.data.vertices[i].co for i in triangle.vertices])
            if plane:bank_triangles.append(plane)
    records=[]
    for obj in logs:
        obj.data.calc_loop_triangles()
        for delta in (0,.1,.25,.5,1,2,4,8,16,32,48):
            shift=Vector((0,-COS/SIN*delta,delta));minimum=min(v.co.z+delta for v in obj.data.vertices)
            for triangle in obj.data.loop_triangles:
                plane=planar_triangle([obj.matrix_world@obj.data.vertices[i].co+shift for i in triangle.vertices])
                if plane is None:continue
                points,normal,offset=plane
                for bank_points,bank_normal,bank_offset in bank_triangles:
                    if any(max(p[axis]for p in points)<min(p[axis]for p in bank_points)or max(p[axis]for p in bank_points)<min(p[axis]for p in points)for axis in(0,1)):continue
                    intersection=overlap(points,bank_points)
                    area=abs(sum(a[0]*b[1]-b[0]*a[1]for a,b in zip(intersection,intersection[1:]+intersection[:1])))/2
                    if area<1e-5:continue
                    for x,y in intersection:
                        minimum=min(minimum,(offset-normal[0]*x-normal[1]*y)/normal[2]-(bank_offset-bank_normal[0]*x-bank_normal[1]*y)/bank_normal[2])
            row=dict(object=obj.name,source_ray_height_delta=delta,minimum_clearance=minimum);records.append(row);print(row,flush=True)
    (base/'cliff-ray-lift-probe.json').write_text(json.dumps(dict(status='diagnostic only; no model saved',model_sha256=report['model_sha256'],bank_model_sha256=report['bank_model_sha256'],records=records),indent=2)+'\n')


if __name__=='__main__':main()

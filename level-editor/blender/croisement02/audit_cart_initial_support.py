"""Check saved cart solid clearances against exact bank and the ground plane."""
import json,sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from audit_log_endpoint_contact import planar_triangle,overlap,sha

def main():
    base=OUT/'north-cart-initial-candidate-v3';manifest=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==manifest['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));objects=[bpy.data.objects[name]for name in manifest['objects']];bank=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank';audit=json.loads((bank/'inspection/saved-model-audit.json').read_text());assert sha(bank/'model.blend')==audit['model_sha256'];names=[r['object']for r in audit['objects']if r['source_node']in[f'building-{i:03d}'for i in range(5)]]
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False)as(src,dst):dst.objects=names
    for obj in dst.objects:bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.update();bank_planes=[]
    for obj in dst.objects:
        obj.data.calc_loop_triangles()
        for t in obj.data.loop_triangles:
            plane=planar_triangle([obj.matrix_world@obj.data.vertices[i].co for i in t.vertices])
            if plane:bank_planes.append(plane)
    records=[]
    for obj in objects:
        vertices=[obj.matrix_world@v.co for v in obj.data.vertices];minimum=min(p.z for p in vertices);low=np.min(np.array(vertices),axis=0);high=np.max(np.array(vertices),axis=0);candidates=[p for p in bank_planes if not any(high[a]<min(v[a]for v in p[0])or max(v[a]for v in p[0])<low[a]for a in(0,1))];intersections=0
        if candidates:
            obj.data.calc_loop_triangles()
            for t in obj.data.loop_triangles:
                plane=planar_triangle([vertices[i]for i in t.vertices])
                if not plane:continue
                points,n,c=plane
                for bp,bn,bc in candidates:
                    if any(max(p[a]for p in points)<min(p[a]for p in bp)or max(p[a]for p in bp)<min(p[a]for p in points)for a in(0,1)):continue
                    polygon=overlap(points,bp);area=abs(sum(a[0]*b[1]-b[0]*a[1]for a,b in zip(polygon,polygon[1:]+polygon[:1])))/2
                    if area<1e-5:continue
                    intersections+=1
                    for x,y in polygon:minimum=min(minimum,(c-n[0]*x-n[1]*y)/n[2]-(bc-bn[0]*x-bn[1]*y)/bn[2])
        records.append(dict(object=obj.name,minimum_clearance=minimum,bank_triangle_candidates=len(candidates),bank_overlaps=intersections,world_bounds=[low.tolist(),high.tolist()]))
    wheels=[r for r in records if r['object'].startswith('Wheel rim')];passed=all(r['minimum_clearance']>=-.05 for r in records)and all(abs(r['minimum_clearance'])<.1 for r in wheels);report=dict(status='PASS saved cart support'if passed else'HOLD',model_sha256=manifest['model_sha256'],bank_model_sha256=audit['model_sha256'],objects=records,limitations=['Initial target pose only; mobile road and later breakup contacts are separate.','Ground and bank support does not approve geometry proportions or source ownership.']);(base/'support-audit.json').write_text(json.dumps(report,indent=2)+'\n');print(report['status'],[(r['object'],r['minimum_clearance'],r['bank_triangle_candidates'])for r in records])
if __name__=='__main__':main()

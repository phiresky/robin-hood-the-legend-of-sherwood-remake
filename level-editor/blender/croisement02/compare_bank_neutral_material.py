"""Read-only latest bank material samples at frozen neutral-component pixels."""
import json
from pathlib import Path
import sys
from collections import Counter
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY


def main():
    evidence=OUT/'restart2-vegetation/neutral-first-hit-v1/audit.json';frozen=json.loads(evidence.read_text())
    model=OUT/'restart2-bank321/foot-candidate-v2/worker.blend';output=OUT/'restart2-bank321/neutral-material-comparison-v1'
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    domain_path=OUT/'terrain-bank-candidate/bank-source-domain.png';domain=np.asarray(Image.open(domain_path).convert('L'))>0
    samples=[dict(s,component=c['component']) for c in frozen['components'] for s in c['samples'] if s['asset']=='croisement02-north-woodland-bank']
    if len(samples)!=4867:raise ValueError('Frozen sample scope changed')
    model_hash=sha(model);acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
        vertices=[];triangles=[];records=[];cache={}
        for obj in objects:
            start=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);obj.data.calc_loop_triangles()
            for tri in obj.data.loop_triangles:
                triangles.append(tuple(start+i for i in tri.vertices));mat=obj.data.materials[tri.material_index]
                node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
                uv_name=node.inputs['Vector'].links[0].from_node.uv_map if node.inputs['Vector'].links and hasattr(node.inputs['Vector'].links[0].from_node,'uv_map') else obj.data.uv_layers.active.name
                uv=np.array([obj.data.uv_layers[uv_name].data[i].uv[:] for i in tri.loops])
                image=node.image
                if image.name not in cache:
                    pixels=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(pixels);cache[image.name]=pixels.reshape(image.size[1],image.size[0],4)
                records.append((obj.name,mat.name,image.name,uv,node.interpolation))
        tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True);world=np.array(vertices)
        counts=Counter();rows=[]
        for sample in samples:
            x,y=sample['pixel'];p,_,index,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+Vector(RAY)*10000,-Vector(RAY))
            owned=bool(domain[y,x]);row=dict(pixel=[x,y],component=sample['component'],native_bank_owned=owned)
            if p is None:row['classification']='no-bank-hit'
            else:
                name,material,image,uv,interpolation=records[index];points=world[list(triangles[index])]
                weights=np.linalg.lstsq(np.column_stack((points[1]-points[0],points[2]-points[0])),np.array(p)-points[0],rcond=None)[0]
                coord=np.array([1-weights.sum(),*weights])@uv;pixels=cache[image];h,w=pixels.shape[:2]
                ix=int(np.clip(np.floor(coord[0]*w),0,w-1));iy=int(np.clip(np.floor(coord[1]*h),0,h-1));color=pixels[iy,ix,:3]
                neutral=bool(color.max()-color.min()<1/255)
                row.update(object=name,material=material,image=image,uv=coord.tolist(),sample_rgba=pixels[iy,ix].tolist(),sampling='nearest atlas texel; not shaded multisample render',material_interpolation=interpolation,classification='neutral-atlas-sample' if neutral else 'colored-atlas-sample')
            counts[(row['classification'],'native-bank-owned' if owned else 'excluded-other-source')]+=1;rows.append(row)
        source=Image.open(OUT/'ground-receiver-review-v5/reference/source.png').convert('RGB');marked=np.array(source)
        for row in rows: x,y=row['pixel'];marked[y,x]=(0,220,180) if row['native_bank_owned'] else (220,60,180)
        Image.fromarray(marked).save(output/'source-ownership.png')
        write_json(output/'report.json',dict(status='Read-only material comparison; no role changes',model_sha256=model_hash,frozen_audit_sha256=sha(evidence),bank_domain_sha256=sha(domain_path),samples=len(rows),counts=[dict(classification=a,role=b,pixels=n) for (a,b),n in sorted(counts.items())],rows=rows,limitations=['Only bank geometry is queried at the frozen4867 positions; this is not a new whole-scene first-hit audit.','A colored replacement atlas sample does not resolve missing foreground geometry or grant bank source ownership.','Nearest texture texel classification differs from shaded multisample rendered gray detection.','Foot correction and material-role completion are independent.'],geometry_changed=False,source_ownership_changed=False))
        if sha(model)!=model_hash:raise ValueError('Model changed during read-only comparison')
        print(dict((str(k),v) for k,v in counts.items()))
    finally:release()


if __name__=='__main__':main()

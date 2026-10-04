"""Separate inferred-boundary mesh coverage from material-alpha coverage."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from catalog import OUT,tree_workspace
from tree_geometry import SIN,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    old=json.loads((OUT/'understory-candidates/mixed75-91-source-v2/receiver-coverage/report.json').read_text())
    output=OUT/'mixed-boundary-mesh-audit-v1';output.mkdir(exist_ok=False)
    rows=[]
    for entry in old['records']:
        if not entry['receiver'].startswith('wood'):continue
        index=int(entry['receiver'][4:]);worker=tree_workspace(index);digest=sha(worker/'model.blend')
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update()
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown']
        if not objects:raise ValueError('Missing receiver meshes')
        surfaces=[(o,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices) for p in o.data.polygons])) for o in objects]
        domain=Path(entry['mask']);mask=np.asarray(Image.open(domain).convert('L'))>0
        if sha(domain)!=entry['mask_sha256']:raise ValueError('Boundary changed')
        hitmask=np.zeros_like(mask);samples=[]
        for y,x in zip(*np.nonzero(mask)):
            origin=Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000;hits=[]
            for obj,surface in surfaces:
                location,normal,face,distance=surface.ray_cast(origin,-RAY)
                if location is not None:hits.append((distance,obj,location,normal,face))
            if hits:
                distance,obj,location,normal,face=min(hits,key=lambda h:h[0]);hitmask[y,x]=True
                material=obj.data.materials[obj.data.polygons[face].material_index]
                samples.append(dict(source=[int(x),int(y)],object=obj.name,source_node=obj.get('source_node'),point=list(location),normal=list(normal),material=material.name,front_facing=normal.dot(RAY)>0))
        Image.fromarray(hitmask.astype('uint8')*255).save(output/f'{index}-mesh-hits.png')
        classes=[]
        if index==38:
            for name in ('trunk-basal-contour','distal-root-inference'):
                classmask=np.asarray(Image.open(OUT/f'tree38-root-research/{name}.png').convert('L'))>0
                classes.append(dict(role=name,pixels=int(classmask.sum()),mesh_hits=int((classmask&hitmask).sum()),mesh_misses=int((classmask&~hitmask).sum())))
        if sha(worker/'model.blend')!=digest:raise ValueError('Audit modified model')
        rows.append(dict(receiver=entry['receiver'],model=str(worker/'model.blend'),model_sha256=digest,mask_sha256=sha(domain),pixels=int(mask.sum()),mesh_hits=int(hitmask.sum()),mesh_misses=int((mask&~hitmask).sum()),previous_actual_alpha_covered=entry['covered_pixels'],classes=classes,samples=samples,limitation='Opaque mesh intersections ignore material alpha and do not establish observed material or final scene first-hit ownership.'))
    write_json(output/'report.json',dict(read_only=True,records=rows,status='Diagnostic evidence only; no source-role or model changes'))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
